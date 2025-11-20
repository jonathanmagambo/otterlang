use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use anyhow::{Context, Result, anyhow, bail};
use cranelift_codegen::{ir, ir::InstBuilder};
use cranelift_frontend::FunctionBuilder;

use ast::nodes::{BinaryOp, Block, Expr, FStringPart, Literal, Pattern, Statement, UnaryOp};

use super::backend::CraneliftBackend;

impl CraneliftBackend {
    /// Lower a block of statements to Cranelift IR
    fn lower_block_with_builder(
        &mut self,
        builder: &mut FunctionBuilder,
        block: &Block,
        variables: &mut HashMap<String, ir::Value>,
    ) -> Result<ir::Value> {
        let mut last_value = None;

        for stmt in &block.statements {
            match stmt {
                Statement::Let {
                    name, ty: _, expr, ..
                } => {
                    let val = self.lower_expr_with_builder(builder, expr, variables)?;
                    variables.insert(name.clone(), val);
                    last_value = Some(val);
                }
                Statement::Return(expr) => {
                    if let Some(expr) = expr {
                        let val = self.lower_expr_with_builder(builder, expr, variables)?;
                        builder.ins().return_(&[val]);
                    } else {
                        // Return void
                        builder.ins().return_(&[]);
                    }
                    // For simplicity, return a dummy value after return statement
                    // In a full implementation, we'd need to handle control flow properly
                    last_value = Some(builder.ins().iconst(ir::types::I64, 0));
                }
                Statement::Function(_) => {
                    // Function declarations are handled at module level
                    // Skip here
                }
                _ => bail!("Statement type not yet implemented: {:?}", stmt),
            }
        }

        // Return the last computed value, or a default if no statements
        Ok(last_value.unwrap_or_else(|| builder.ins().iconst(ir::types::I64, 0)))
    }

    /// Lower an expression to Cranelift IR
    fn lower_expr_with_builder(
        &mut self,
        builder: &mut FunctionBuilder,
        expr: &Expr,
        variables: &HashMap<String, ir::Value>,
    ) -> Result<ir::Value> {
        match expr {
            Expr::Literal(lit) => self.lower_literal_with_builder(builder, lit),
            Expr::Identifier { name, .. } => variables
                .get(name)
                .copied()
                .ok_or_else(|| anyhow!("Undefined variable: {}", name)),
            Expr::Binary { left, op, right } => {
                let left_val = self.lower_expr_with_builder(builder, left, variables)?;
                let right_val = self.lower_expr_with_builder(builder, right, variables)?;
                Self::lower_binary_op_with_builder(builder, *op, left_val, right_val)
            }
            Expr::Call { func, args } => {
                self.lower_call_with_builder(builder, func, args, variables)
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.lower_if_expr_with_builder(builder, cond, then_branch, else_branch, variables)
            }
            Expr::Unary { op, expr } => {
                let val = self.lower_expr_with_builder(builder, expr, variables)?;
                Self::lower_unary_op_with_builder(builder, *op, val)
            }
            Expr::Member { object, field } => {
                let object_val = self.lower_expr_with_builder(builder, object, variables)?;

                // For member access, we assume the object is a pointer to a struct
                // and we need to calculate the field offset.
                // In a real implementation, this would look up the struct definition
                // and calculate proper field offsets based on the struct layout.

                // For now, use a simple approach: assume fields are at fixed offsets
                // This is highly simplified and would need proper struct layout knowledge

                // Calculate field offset (simplified: assume each field is pointer-sized)
                let field_offset = match field.as_str() {
                    // Common field names get predefined offsets
                    "x" | "0" => 0,  // First field
                    "y" | "1" => 8,  // Second field (assuming 64-bit pointers)
                    "z" | "2" => 16, // Third field
                    _ => {
                        // For unknown fields, use a hash-based offset (not ideal but deterministic)
                        use std::collections::hash_map::DefaultHasher;
                        use std::hash::{Hash, Hasher};
                        let mut hasher = DefaultHasher::new();
                        field.hash(&mut hasher);
                        (hasher.finish() % 100 * 8) as i32 // Limit to reasonable offset
                    }
                };

                // Calculate address of field: object_ptr + offset
                let offset_val = builder.ins().iconst(ir::types::I64, field_offset as i64);
                let field_addr = builder.ins().iadd(object_val, offset_val);

                // Load the field value (assume it's a pointer-sized value)
                let field_value =
                    builder
                        .ins()
                        .load(ir::types::I64, ir::MemFlags::new(), field_addr, 0);

                Ok(field_value)
            }
            Expr::Array(elements) => {
                if elements.is_empty() {
                    // Empty array - return null pointer for now
                    Ok(builder.ins().iconst(ir::types::I64, 0))
                } else {
                    // Allocate memory for the array
                    let num_elements = elements.len();
                    let element_size = 8; // Assume pointer-sized elements for now
                    let total_size = num_elements * element_size;

                    // Call external allocation function
                    let malloc_func_id = if let Some(&id) = self.declared_functions.get("malloc") {
                        id
                    } else {
                        // Declare malloc: fn(size: usize) -> *mut u8
                        let mut sig = ir::Signature::new(self.isa.default_call_conv());
                        sig.params.push(ir::AbiParam::new(ir::types::I64)); // size parameter
                        sig.returns.push(ir::AbiParam::new(
                            self.module.target_config().pointer_type(),
                        ));

                        let func_id = self
                            .module
                            .declare_function("malloc", cranelift_module::Linkage::Import, &sig)
                            .map_err(|e| anyhow!("Failed to declare malloc function: {}", e))?;

                        self.declared_functions
                            .insert("malloc".to_string(), func_id);
                        func_id
                    };

                    // Allocate memory
                    let size_val = builder.ins().iconst(ir::types::I64, total_size as i64);
                    let malloc_call = self
                        .module
                        .declare_func_in_func(malloc_func_id, builder.func);
                    let array_ptr = builder.ins().call(malloc_call, &[size_val]);

                    // Initialize array elements
                    for (i, element) in elements.iter().enumerate() {
                        let element_val =
                            self.lower_expr_with_builder(builder, element, variables)?;

                        // Calculate offset for this element
                        let offset = (i * element_size) as i32;
                        let element_addr = builder.ins().iadd(
                            array_ptr,
                            builder.ins().iconst(ir::types::I64, offset as i64),
                        );

                        // Store element value
                        builder
                            .ins()
                            .store(ir::MemFlags::new(), element_val, element_addr, 0);
                    }

                    Ok(array_ptr)
                }
            }
            Expr::Dict(pairs) => {
                if pairs.is_empty() {
                    // Empty dict - return null pointer for now
                    Ok(builder.ins().iconst(ir::types::I64, 0))
                } else {
                    // Allocate memory for the dictionary
                    // Structure: [key1, value1, key2, value2, ...]
                    let num_pairs = pairs.len();
                    let entry_size = 16; // key + value (both pointer-sized)
                    let total_size = num_pairs * entry_size;

                    // Call external allocation function
                    let malloc_func_id = if let Some(&id) = self.declared_functions.get("malloc") {
                        id
                    } else {
                        // Declare malloc: fn(size: usize) -> *mut u8
                        let mut sig = ir::Signature::new(self.isa.default_call_conv());
                        sig.params.push(ir::AbiParam::new(ir::types::I64)); // size parameter
                        sig.returns.push(ir::AbiParam::new(
                            self.module.target_config().pointer_type(),
                        ));

                        let func_id = self
                            .module
                            .declare_function("malloc", cranelift_module::Linkage::Import, &sig)
                            .map_err(|e| anyhow!("Failed to declare malloc function: {}", e))?;

                        self.declared_functions
                            .insert("malloc".to_string(), func_id);
                        func_id
                    };

                    // Allocate memory
                    let size_val = builder.ins().iconst(ir::types::I64, total_size as i64);
                    let malloc_call = self
                        .module
                        .declare_func_in_func(malloc_func_id, builder.func);
                    let dict_ptr = builder.ins().call(malloc_call, &[size_val]);

                    // Initialize dictionary entries
                    for (i, (key_expr, value_expr)) in pairs.iter().enumerate() {
                        let key_val = self.lower_expr_with_builder(builder, key_expr, variables)?;
                        let value_val =
                            self.lower_expr_with_builder(builder, value_expr, variables)?;

                        // Calculate offsets for key and value in this entry
                        let entry_offset = i * entry_size;
                        let key_offset = entry_offset;
                        let value_offset = entry_offset + 8;

                        // Store key
                        let key_addr = builder.ins().iadd(
                            dict_ptr,
                            builder.ins().iconst(ir::types::I64, key_offset as i64),
                        );
                        builder
                            .ins()
                            .store(ir::MemFlags::new(), key_val, key_addr, 0);

                        // Store value
                        let value_addr = builder.ins().iadd(
                            dict_ptr,
                            builder.ins().iconst(ir::types::I64, value_offset as i64),
                        );
                        builder
                            .ins()
                            .store(ir::MemFlags::new(), value_val, value_addr, 0);
                    }

                    Ok(dict_ptr)
                }
            }
            Expr::Struct { name: _, fields } => {
                // Allocate memory for the struct
                let num_fields = fields.len();
                if num_fields == 0 {
                    // Empty struct - return null pointer
                    Ok(builder.ins().iconst(ir::types::I64, 0))
                } else {
                    let field_size = 8; // Assume pointer-sized fields for now
                    let total_size = num_fields * field_size;

                    // Call external allocation function
                    let malloc_func_id = if let Some(&id) = self.declared_functions.get("malloc") {
                        id
                    } else {
                        // Declare malloc: fn(size: usize) -> *mut u8
                        let mut sig = ir::Signature::new(self.isa.default_call_conv());
                        sig.params.push(ir::AbiParam::new(ir::types::I64)); // size parameter
                        sig.returns.push(ir::AbiParam::new(
                            self.module.target_config().pointer_type(),
                        ));

                        let func_id = self
                            .module
                            .declare_function("malloc", cranelift_module::Linkage::Import, &sig)
                            .map_err(|e| anyhow!("Failed to declare malloc function: {}", e))?;

                        self.declared_functions
                            .insert("malloc".to_string(), func_id);
                        func_id
                    };

                    // Allocate memory
                    let size_val = builder.ins().iconst(ir::types::I64, total_size as i64);
                    let malloc_call = self
                        .module
                        .declare_func_in_func(malloc_func_id, builder.func);
                    let struct_ptr = builder.ins().call(malloc_call, &[size_val]);

                    // Initialize struct fields
                    for (i, (field_name, field_expr)) in fields.iter().enumerate() {
                        let field_val =
                            self.lower_expr_with_builder(builder, field_expr, variables)?;

                        // Calculate field offset (use same logic as member access)
                        let field_offset = match field_name.as_str() {
                            "x" | "0" => 0,
                            "y" | "1" => 8,
                            "z" | "2" => 16,
                            _ => {
                                // For unknown fields, use index-based offset
                                i * field_size
                            }
                        };

                        // Store field value
                        let field_addr = builder.ins().iadd(
                            struct_ptr,
                            builder.ins().iconst(ir::types::I64, field_offset as i64),
                        );
                        builder
                            .ins()
                            .store(ir::MemFlags::new(), field_val, field_addr, 0);
                    }

                    Ok(struct_ptr)
                }
            }
            Expr::Match { value, arms } => {
                // Evaluate the value to match against
                let match_value = self.lower_expr_with_builder(builder, value, variables)?;

                // Create blocks for each arm and a default block
                let mut arm_blocks = Vec::new();
                let mut arm_end_blocks = Vec::new();
                let merge_block = builder.create_block();
                let result_type = ir::types::I64; // Assume all match arms return the same type
                builder.append_block_param(merge_block, result_type);

                for _ in arms {
                    arm_blocks.push(builder.create_block());
                    arm_end_blocks.push(builder.create_block());
                }

                // Create a default block for when no pattern matches
                let default_block = builder.create_block();
                let default_val = builder.ins().iconst(ir::types::I64, 0); // Default value when no match
                builder.switch_to_block(default_block);
                builder.ins().jump(merge_block, &[default_val]);

                // Process each arm
                for (i, arm) in arms.iter().enumerate() {
                    builder.switch_to_block(arm_blocks[i]);

                    // Check if pattern matches
                    let pattern_matches =
                        self.lower_pattern_match(builder, &arm.pattern, match_value, variables)?;

                    // If there's a guard, also check it
                    let guard_passes = if let Some(guard) = &arm.guard {
                        let guard_val = self.lower_expr_with_builder(builder, guard, variables)?;
                        // Convert to boolean (non-zero = true)
                        let zero = builder.ins().iconst(ir::types::I8, 0);
                        builder
                            .ins()
                            .icmp(ir::condcodes::IntCC::NotEqual, guard_val, zero)
                    } else {
                        // No guard, always true
                        builder.ins().iconst(ir::types::I8, 1)
                    };

                    // Both pattern and guard must pass
                    let condition = builder.ins().band(pattern_matches, guard_passes);

                    // If condition is true, execute body; else try next arm or default
                    let next_block = if i + 1 < arms.len() {
                        arm_blocks[i + 1]
                    } else {
                        default_block
                    };

                    builder
                        .ins()
                        .brif(condition, arm_end_blocks[i], &[], next_block, &[]);

                    // Execute the arm body
                    builder.switch_to_block(arm_end_blocks[i]);
                    let arm_result = self.lower_expr_with_builder(builder, &arm.body, variables)?;
                    builder.ins().jump(merge_block, &[arm_result]);
                }

                // Start with the first arm
                if !arms.is_empty() {
                    builder.switch_to_block(arm_blocks[0]);
                    builder.ins().jump(arm_blocks[0], &[]);
                }

                // Seal all blocks
                for block in &arm_blocks {
                    builder.seal_block(*block);
                }
                for block in &arm_end_blocks {
                    builder.seal_block(*block);
                }
                builder.seal_block(default_block);
                builder.seal_block(merge_block);

                // Return the result
                builder.switch_to_block(merge_block);
                let result = builder.block_params(merge_block)[0];
                Ok(result)
            }
            Expr::Range { start, end } => {
                // Create a range object with start and end values
                // Structure: [start, end] (two pointer-sized values)

                let start_val = self.lower_expr_with_builder(builder, start, variables)?;
                let end_val = self.lower_expr_with_builder(builder, end, variables)?;

                // Allocate memory for range struct (start + end)
                let range_size = 16; // Two pointer-sized values

                // Call external allocation function
                let malloc_func_id = if let Some(&id) = self.declared_functions.get("malloc") {
                    id
                } else {
                    // Declare malloc: fn(size: usize) -> *mut u8
                    let mut sig = ir::Signature::new(self.isa.default_call_conv());
                    sig.params.push(ir::AbiParam::new(ir::types::I64)); // size parameter
                    sig.returns.push(ir::AbiParam::new(
                        self.module.target_config().pointer_type(),
                    ));

                    let func_id = self
                        .module
                        .declare_function("malloc", cranelift_module::Linkage::Import, &sig)
                        .map_err(|e| anyhow!("Failed to declare malloc function: {}", e))?;

                    self.declared_functions
                        .insert("malloc".to_string(), func_id);
                    func_id
                };

                // Allocate memory
                let size_val = builder.ins().iconst(ir::types::I64, range_size);
                let malloc_call = self
                    .module
                    .declare_func_in_func(malloc_func_id, builder.func);
                let range_ptr = builder.ins().call(malloc_call, &[size_val]);

                // Store start value at offset 0
                let start_addr = range_ptr;
                builder
                    .ins()
                    .store(ir::MemFlags::new(), start_val, start_addr, 0);

                // Store end value at offset 8
                let end_addr = builder
                    .ins()
                    .iadd(range_ptr, builder.ins().iconst(ir::types::I64, 8));
                builder
                    .ins()
                    .store(ir::MemFlags::new(), end_val, end_addr, 0);

                Ok(range_ptr)
            }
            Expr::ListComprehension {
                element,
                var,
                iterable,
                condition,
            } => {
                // Implement proper list comprehension with iteration
                // This handles array iteration with dynamic result collection

                // Evaluate the iterable (assume it's an array for now)
                let iterable_val = self.lower_expr_with_builder(builder, iterable, variables)?;

                // Create loop structure for iteration
                let loop_header = builder.create_block();
                let loop_body = builder.create_block();
                let loop_exit = builder.create_block();
                let result_merge = builder.create_block();
                builder
                    .append_block_param(result_merge, self.module.target_config().pointer_type());

                // Initialize loop variables
                let index_var = builder.ins().iconst(ir::types::I64, 0); // Current index
                let result_ptr_var = builder.ins().iconst(ir::types::I64, 0); // Result array (initially null)
                let result_count_var = builder.ins().iconst(ir::types::I64, 0); // Number of elements added

                // Jump to loop header
                builder
                    .ins()
                    .jump(loop_header, &[index_var, result_ptr_var, result_count_var]);

                // Loop header: check if we've reached the end
                builder.switch_to_block(loop_header);
                let current_index = builder.block_params(loop_header)[0];
                let current_result_ptr = builder.block_params(loop_header)[1];
                let current_result_count = builder.block_params(loop_header)[2];

                // Read the array length from the first element of the array structure
                let length_addr = iterable_val; // Length is at offset 0
                let array_length =
                    builder
                        .ins()
                        .load(ir::types::I64, ir::MemFlags::new(), length_addr, 0);

                let continue_loop = builder.ins().icmp(
                    ir::condcodes::IntCC::SignedLessThan,
                    current_index,
                    array_length,
                );
                builder.ins().brif(
                    continue_loop,
                    loop_body,
                    &[],
                    loop_exit,
                    &[current_result_ptr],
                );

                // Loop body: process each element
                builder.switch_to_block(loop_body);

                // Load current element from iterable array
                // Array structure: [length, element0, element1, ...] so elements start at offset 8
                let element_offset = builder.ins().iadd(
                    builder.ins().iconst(ir::types::I64, 8), // Skip length field
                    builder
                        .ins()
                        .imul(current_index, builder.ins().iconst(ir::types::I64, 8)),
                );
                let element_addr = builder.ins().iadd(iterable_val, element_offset);
                let current_element =
                    builder
                        .ins()
                        .load(ir::types::I64, ir::MemFlags::new(), element_addr, 0);

                // Bind the element to the variable
                variables.insert(var.clone(), current_element);

                // Check condition if present
                let condition_met = if let Some(cond) = condition {
                    let cond_val = self.lower_expr_with_builder(builder, cond, variables)?;
                    // Convert to boolean (non-zero means true)
                    builder.ins().icmp(
                        ir::condcodes::IntCC::NotEqual,
                        cond_val,
                        builder.ins().iconst(ir::types::I64, 0),
                    )
                } else {
                    builder.ins().iconst(ir::types::I8, 1) // No condition, always true
                };

                // Create blocks for conditional element addition
                let add_element_block = builder.create_block();
                let skip_element_block = builder.create_block();
                let element_merge_block = builder.create_block();
                builder.append_block_param(
                    element_merge_block,
                    self.module.target_config().pointer_type(),
                );
                builder.append_block_param(element_merge_block, ir::types::I64);

                builder.ins().brif(
                    condition_met,
                    add_element_block,
                    &[],
                    skip_element_block,
                    &[],
                );

                // Add element block: evaluate element expression and add to result
                builder.switch_to_block(add_element_block);
                let elem_val = self.lower_expr_with_builder(builder, element, variables)?;

                // Grow result array or create new one
                let (new_result_ptr, new_result_count) = self.grow_result_array(
                    builder,
                    current_result_ptr,
                    current_result_count,
                    elem_val,
                )?;

                builder
                    .ins()
                    .jump(element_merge_block, &[new_result_ptr, new_result_count]);

                // Skip element block: keep current result
                builder.switch_to_block(skip_element_block);
                builder.ins().jump(
                    element_merge_block,
                    &[current_result_ptr, current_result_count],
                );

                // Element merge: continue loop with updated result
                builder.switch_to_block(element_merge_block);
                let updated_result_ptr = builder.block_params(element_merge_block)[0];
                let updated_result_count = builder.block_params(element_merge_block)[1];

                let next_index = builder
                    .ins()
                    .iadd(current_index, builder.ins().iconst(ir::types::I64, 1));
                builder.ins().jump(
                    loop_header,
                    &[next_index, updated_result_ptr, updated_result_count],
                );

                // Loop exit: return final result
                builder.switch_to_block(loop_exit);
                let final_result = builder.block_params(loop_exit)[0];
                builder.ins().jump(result_merge, &[final_result]);

                // Result merge and return
                builder.switch_to_block(result_merge);
                let result = builder.block_params(result_merge)[0];

                // Seal all blocks
                builder.seal_block(loop_header);
                builder.seal_block(loop_body);
                builder.seal_block(loop_exit);
                builder.seal_block(add_element_block);
                builder.seal_block(skip_element_block);
                builder.seal_block(element_merge_block);
                builder.seal_block(result_merge);

                Ok(result)
            }
            Expr::DictComprehension {
                key,
                value,
                var,
                iterable,
                condition,
            } => {
                // Lower dict comprehensions using a loop similar to list comprehensions.
                let iterable_val = self.lower_expr_with_builder(builder, iterable, variables)?;

                // Create loop structure
                let loop_header = builder.create_block();
                let loop_body = builder.create_block();
                let loop_exit = builder.create_block();
                let result_merge = builder.create_block();
                let pointer_type = self.module.target_config().pointer_type();

                builder.append_block_param(loop_header, ir::types::I64); // index
                builder.append_block_param(loop_header, pointer_type); // dict pointer
                builder.append_block_param(loop_header, ir::types::I64); // entry count
                builder.append_block_param(result_merge, pointer_type);

                // Initialize loop state
                let start_index = builder.ins().iconst(ir::types::I64, 0);
                let start_dict = builder.ins().iconst(pointer_type, 0);
                let start_count = builder.ins().iconst(ir::types::I64, 0);
                builder
                    .ins()
                    .jump(loop_header, &[start_index, start_dict, start_count]);

                // Loop header
                builder.switch_to_block(loop_header);
                let current_index = builder.block_params(loop_header)[0];
                let current_dict_ptr = builder.block_params(loop_header)[1];
                let current_count = builder.block_params(loop_header)[2];

                // Assume iterable arrays store length at offset 0.
                let length_addr = iterable_val;
                let array_length =
                    builder
                        .ins()
                        .load(ir::types::I64, ir::MemFlags::new(), length_addr, 0);
                let continue_loop = builder.ins().icmp(
                    ir::condcodes::IntCC::SignedLessThan,
                    current_index,
                    array_length,
                );
                builder.ins().brif(
                    continue_loop,
                    loop_body,
                    &[],
                    loop_exit,
                    &[current_dict_ptr],
                );

                // Loop body
                builder.switch_to_block(loop_body);

                // Load iterable element (offset 8 + index * 8)
                let element_offset = builder.ins().iadd(
                    builder.ins().iconst(ir::types::I64, 8),
                    builder
                        .ins()
                        .imul(current_index, builder.ins().iconst(ir::types::I64, 8)),
                );
                let element_addr = builder.ins().iadd(iterable_val, element_offset);
                let current_element =
                    builder
                        .ins()
                        .load(ir::types::I64, ir::MemFlags::new(), element_addr, 0);
                variables.insert(var.clone(), current_element);

                // Evaluate optional condition
                let condition_met = if let Some(cond) = condition {
                    let cond_val = self.lower_expr_with_builder(builder, cond, variables)?;
                    builder.ins().icmp(
                        ir::condcodes::IntCC::NotEqual,
                        cond_val,
                        builder.ins().iconst(ir::types::I64, 0),
                    )
                } else {
                    builder.ins().iconst(ir::types::I8, 1)
                };

                let add_entry_block = builder.create_block();
                let skip_entry_block = builder.create_block();
                let entry_merge_block = builder.create_block();
                builder.append_block_param(entry_merge_block, pointer_type);
                builder.append_block_param(entry_merge_block, ir::types::I64);

                builder
                    .ins()
                    .brif(condition_met, add_entry_block, &[], skip_entry_block, &[]);

                // Add entry block
                builder.switch_to_block(add_entry_block);
                let key_val = self.lower_expr_with_builder(builder, key, variables)?;
                let value_val = self.lower_expr_with_builder(builder, value, variables)?;
                let (new_dict_ptr, new_count) = self.grow_result_dict(
                    builder,
                    current_dict_ptr,
                    current_count,
                    key_val,
                    value_val,
                )?;
                builder
                    .ins()
                    .jump(entry_merge_block, &[new_dict_ptr, new_count]);

                // Skip entry block
                builder.switch_to_block(skip_entry_block);
                builder
                    .ins()
                    .jump(entry_merge_block, &[current_dict_ptr, current_count]);

                // Merge entry results and continue loop
                builder.switch_to_block(entry_merge_block);
                let merged_dict_ptr = builder.block_params(entry_merge_block)[0];
                let merged_count = builder.block_params(entry_merge_block)[1];
                let next_index = builder
                    .ins()
                    .iadd(current_index, builder.ins().iconst(ir::types::I64, 1));
                builder
                    .ins()
                    .jump(loop_header, &[next_index, merged_dict_ptr, merged_count]);

                // Loop exit
                builder.switch_to_block(loop_exit);
                let final_dict_ptr = builder.block_params(loop_exit)[0];
                builder.ins().jump(result_merge, &[final_dict_ptr]);

                builder.switch_to_block(result_merge);
                let result = builder.block_params(result_merge)[0];

                builder.seal_block(loop_header);
                builder.seal_block(loop_body);
                builder.seal_block(loop_exit);
                builder.seal_block(add_entry_block);
                builder.seal_block(skip_entry_block);
                builder.seal_block(entry_merge_block);
                builder.seal_block(result_merge);

                Ok(result)
            }
            Expr::FString { parts } => {
                if parts.is_empty() {
                    // Empty F-string, return empty string
                    let empty_str =
                        self.lower_literal_with_builder(builder, &Literal::String("".to_string()))?;
                    Ok(empty_str)
                } else {
                    // For now, implement basic string interpolation by creating a simple concatenated result
                    // In a full implementation, this would use proper string formatting

                    // For simplicity, concatenate all text parts and ignore expressions for now
                    let mut concatenated = String::new();

                    for part in &parts {
                        match part {
                            ast::nodes::FStringPart::Text(text) => {
                                concatenated.push_str(text);
                            }
                            ast::nodes::FStringPart::Expr(_expr) => {
                                // TODO: Implement proper expression evaluation and string conversion
                                concatenated.push_str("<expr>");
                            }
                        }
                    }

                    // Create a string literal with the concatenated result
                    self.lower_literal_with_builder(builder, &Literal::String(concatenated))
                }
            }
            Expr::Lambda {
                params,
                ret_ty,
                body,
            } => {
                // Create an anonymous function for the lambda
                // For now, this creates a function at module level (not a true closure)

                // Generate a unique name for the lambda function
                let lambda_name = format!("lambda_{}", self.declared_functions.len());

                // Build signature for the lambda
                let mut sig = ir::Signature::new(self.isa.default_call_conv());

                // Add parameters
                for param in &params {
                    if let Some(param_ty) = &param.ty {
                        let param_type = self.otter_type_to_cranelift(param_ty)?;
                        sig.params.push(ir::AbiParam::new(param_type));
                    } else {
                        // Default parameter type
                        sig.params.push(ir::AbiParam::new(ir::types::I64));
                    }
                }

                // Add return type
                if let Some(ret_ty) = &ret_ty {
                    let ret_type = self.otter_type_to_cranelift(ret_ty)?;
                    if ret_type != ir::types::INVALID {
                        sig.returns.push(ir::AbiParam::new(ret_type));
                    }
                } else {
                    // Default return type
                    sig.returns.push(ir::AbiParam::new(ir::types::I64));
                }

                // Declare the lambda function
                let lambda_func_id = self
                    .module
                    .declare_function(&lambda_name, cranelift_module::Linkage::Local, &sig)
                    .map_err(|e| anyhow!("Failed to declare lambda function: {}", e))?;

                self.declared_functions
                    .insert(lambda_name.clone(), lambda_func_id);

                // Build the lambda function body
                // Create a new function context for the lambda
                let mut lambda_ctx = Context::new();
                lambda_ctx.func.signature = sig.clone();

                // Take the function builder context temporarily
                let mut fn_builder_ctx = std::mem::take(&mut self.fn_builder_ctx);

                // Create the function builder for the lambda
                let mut lambda_builder =
                    FunctionBuilder::new(&mut lambda_ctx.func, &mut fn_builder_ctx);

                // Create entry block for lambda
                let lambda_entry_block = lambda_builder.create_block();
                lambda_builder.switch_to_block(lambda_entry_block);

                // Set up variables for lambda parameters
                let mut lambda_variables = HashMap::new();

                // Declare lambda parameters as variables
                for (i, param) in params.iter().enumerate() {
                    let param_value = lambda_builder.block_params(lambda_entry_block)[i];
                    lambda_variables.insert(param.name.clone(), param_value);
                }

                // Seal the lambda entry block
                lambda_builder.seal_block(lambda_entry_block);

                // Lower the lambda body
                let lambda_result = Self::lower_block_with_builder(
                    self,
                    &mut lambda_builder,
                    &body,
                    &mut lambda_variables,
                )?;

                // Return the result from the lambda
                lambda_builder.ins().return_(&[lambda_result]);

                // Finalize the lambda function
                lambda_builder.finalize();

                // Restore the function builder context
                self.fn_builder_ctx = fn_builder_ctx;

                // Define the lambda function in the module
                self.module
                    .define_function(lambda_func_id, &mut lambda_ctx)
                    .map_err(|e| anyhow!("Failed to define lambda function: {}", e))?;

                // Get a pointer to the lambda function for the result
                let func_addr = self
                    .module
                    .get_name(&lambda_name)
                    .map_err(|e| anyhow!("Failed to get lambda function address: {}", e))?;

                // Convert the function address to a value
                let lambda_ptr = builder.ins().iconst(ir::types::I64, func_addr as i64);

                Ok(lambda_ptr)
            }
            Expr::Await(expr) => {
                // Await a task/future using tokio runtime
                let task_handle = self.lower_expr_with_builder(builder, expr, variables)?;

                // Call the await_task builtin function
                let await_func_id = if let Some(&id) = self.declared_functions.get("await_task") {
                    id
                } else {
                    // Declare await_task function: fn(task_handle: opaque) -> i64
                    let mut sig = ir::Signature::new(self.isa.default_call_conv());
                    sig.params.push(ir::AbiParam::new(
                        self.module.target_config().pointer_type(),
                    ));
                    sig.returns.push(ir::AbiParam::new(ir::types::I64));

                    let func_id = self
                        .module
                        .declare_function("await_task", cranelift_module::Linkage::Import, &sig)
                        .map_err(|e| anyhow!("Failed to declare await_task function: {}", e))?;

                    self.declared_functions
                        .insert("await_task".to_string(), func_id);
                    func_id
                };

                // Call await_task
                let call = self
                    .module
                    .declare_func_in_func(await_func_id, builder.func);
                let result = builder.ins().call(call, &[task_handle]);

                Ok(result)
            }
            Expr::Spawn(expr) => {
                // Spawn a new async task using tokio runtime
                let task_val = self.lower_expr_with_builder(builder, expr, variables)?;

                // Call the spawn_async builtin function
                let spawn_func_id = if let Some(&id) = self.declared_functions.get("spawn_async") {
                    id
                } else {
                    // Declare spawn_async function: fn(value: i64) -> opaque
                    let mut sig = ir::Signature::new(self.isa.default_call_conv());
                    sig.params.push(ir::AbiParam::new(ir::types::I64));
                    sig.returns.push(ir::AbiParam::new(
                        self.module.target_config().pointer_type(),
                    ));

                    let func_id = self
                        .module
                        .declare_function("spawn_async", cranelift_module::Linkage::Import, &sig)
                        .map_err(|e| anyhow!("Failed to declare spawn_async function: {}", e))?;

                    self.declared_functions
                        .insert("spawn_async".to_string(), func_id);
                    func_id
                };

                // Call spawn_async
                let call = self
                    .module
                    .declare_func_in_func(spawn_func_id, builder.func);
                let task_handle = builder.ins().call(call, &[task_val]);

                Ok(task_handle)
            }
        }
    }

    /// Lower a literal to Cranelift IR
    fn lower_literal_with_builder(
        &mut self,
        builder: &mut FunctionBuilder,
        lit: &Literal,
    ) -> Result<ir::Value> {
        match lit {
            Literal::Number(num) => {
                if num.is_float_literal {
                    Ok(builder.ins().f64const(num.value))
                } else {
                    Ok(builder.ins().iconst(ir::types::I64, num.value as i64))
                }
            }
            Literal::Bool(b) => Ok(builder.ins().iconst(ir::types::I8, if *b { 1 } else { 0 })),
            Literal::String(s) => {
                // Create a data object for the string literal
                let data_name = format!("str_lit_{}", self.declared_data.len());
                let data_id = self.module.declare_data(
                    &data_name,
                    cranelift_module::Linkage::Local,
                    false,
                    false,
                )?;

                // Define the data with the string bytes (including null terminator for C compatibility)
                let mut data_bytes = s.as_bytes().to_vec();
                data_bytes.push(0); // Null terminator

                self.module.define_data(data_id, &data_bytes)?;
                self.declared_data.insert(data_name.clone(), data_id);

                // Return a pointer to the string data
                let global_value = self.module.declare_data_in_func(data_id, builder.func);
                Ok(builder
                    .ins()
                    .global_value(self.module.target_config().pointer_type(), global_value))
            }
            Literal::None => Ok(builder.ins().iconst(ir::types::I64, 0)),
            Literal::Unit => Ok(builder.ins().iconst(ir::types::I64, 0)),
        }
    }

    /// Helper method to grow the result array by adding a new element
    fn grow_result_array(
        &mut self,
        builder: &mut FunctionBuilder,
        current_array: ir::Value,
        current_count: ir::Value,
        new_element: ir::Value,
    ) -> Result<(ir::Value, ir::Value)> {
        let element_size = 8; // Assume 8-byte elements for now

        // Check if we need to create a new array or grow existing one
        let create_new_block = builder.create_block();
        let grow_existing_block = builder.create_block();
        let merge_block = builder.create_block();
        builder.append_block_param(merge_block, self.module.target_config().pointer_type());
        builder.append_block_param(merge_block, ir::types::I64);

        // Check if current array is null (first element)
        let is_null = builder.ins().icmp(
            ir::condcodes::IntCC::Equal,
            current_array,
            builder.ins().iconst(ir::types::I64, 0),
        );
        builder
            .ins()
            .brif(is_null, create_new_block, &[], grow_existing_block, &[]);

        // Create new array block (first element)
        builder.switch_to_block(create_new_block);
        let new_size = element_size;
        let malloc_func_id = if let Some(&id) = self.declared_functions.get("malloc") {
            id
        } else {
            let mut sig = ir::Signature::new(self.isa.default_call_conv());
            sig.params.push(ir::AbiParam::new(ir::types::I64));
            sig.returns.push(ir::AbiParam::new(
                self.module.target_config().pointer_type(),
            ));

            let func_id = self
                .module
                .declare_function("malloc", cranelift_module::Linkage::Import, &sig)
                .map_err(|e| anyhow!("Failed to declare malloc function: {}", e))?;

            self.declared_functions
                .insert("malloc".to_string(), func_id);
            func_id
        };

        let size_val = builder.ins().iconst(ir::types::I64, new_size + 8); // +8 for length field
        let malloc_call = self
            .module
            .declare_func_in_func(malloc_func_id, builder.func);
        let new_array = builder.ins().call(malloc_call, &[size_val]);

        // Store length at offset 0
        let length_addr = new_array;
        builder.ins().store(
            ir::MemFlags::new(),
            builder.ins().iconst(ir::types::I64, 1),
            length_addr,
            0,
        );

        // Store element at offset 8
        let element_addr = builder
            .ins()
            .iadd(new_array, builder.ins().iconst(ir::types::I64, 8));
        builder
            .ins()
            .store(ir::MemFlags::new(), new_element, element_addr, 0);

        let new_count = builder.ins().iconst(ir::types::I64, 1);
        builder.ins().jump(merge_block, &[new_array, new_count]);

        // Grow existing array block
        builder.switch_to_block(grow_existing_block);
        // Create a new larger array with proper length field
        let new_capacity = builder
            .ins()
            .iadd(current_count, builder.ins().iconst(ir::types::I64, 4)); // Add some extra capacity
        let new_array_size = builder.ins().iadd(
            builder.ins().iconst(ir::types::I64, 8), // Length field
            builder.ins().imul(
                new_capacity,
                builder.ins().iconst(ir::types::I64, element_size),
            ),
        );
        let grow_malloc_call = self
            .module
            .declare_func_in_func(malloc_func_id, builder.func);
        let grown_array = builder.ins().call(grow_malloc_call, &[new_array_size]);

        // Store updated length at offset 0
        let new_length = builder
            .ins()
            .iadd(current_count, builder.ins().iconst(ir::types::I64, 1));
        let length_addr = grown_array;
        builder
            .ins()
            .store(ir::MemFlags::new(), new_length, length_addr, 0);

        // Copy existing elements from old array to new array using a loop
        let copy_loop_header = builder.create_block();
        let copy_loop_body = builder.create_block();
        let copy_loop_exit = builder.create_block();

        // Initialize copy index
        let copy_index = builder.ins().iconst(ir::types::I64, 0);

        // Jump to copy loop header
        builder.ins().jump(copy_loop_header, &[copy_index]);

        // Copy loop header: check if we've copied all elements
        builder.switch_to_block(copy_loop_header);
        let current_copy_index = builder.block_params(copy_loop_header)[0];
        let should_continue_copy = builder.ins().icmp(
            ir::condcodes::IntCC::SignedLessThan,
            current_copy_index,
            current_count,
        );
        builder.ins().brif(
            should_continue_copy,
            copy_loop_body,
            &[],
            copy_loop_exit,
            &[],
        );

        // Copy loop body: copy one element
        builder.switch_to_block(copy_loop_body);

        // Calculate source address in old array
        let src_offset = builder.ins().iadd(
            builder.ins().iconst(ir::types::I64, 8), // Skip length field in source
            builder.ins().imul(
                current_copy_index,
                builder.ins().iconst(ir::types::I64, element_size),
            ),
        );
        let src_addr = builder.ins().iadd(current_array, src_offset);
        let element_value = builder
            .ins()
            .load(ir::types::I64, ir::MemFlags::new(), src_addr, 0);

        // Calculate destination address in new array
        let dst_offset = builder.ins().iadd(
            builder.ins().iconst(ir::types::I64, 8), // Skip length field in destination
            builder.ins().imul(
                current_copy_index,
                builder.ins().iconst(ir::types::I64, element_size),
            ),
        );
        let dst_addr = builder.ins().iadd(grown_array, dst_offset);
        builder
            .ins()
            .store(ir::MemFlags::new(), element_value, dst_addr, 0);

        // Increment copy index and loop
        let next_copy_index = builder
            .ins()
            .iadd(current_copy_index, builder.ins().iconst(ir::types::I64, 1));
        builder.ins().jump(copy_loop_header, &[next_copy_index]);

        // Copy loop exit: now add the new element
        builder.switch_to_block(copy_loop_exit);
        let insert_offset = builder.ins().iadd(
            builder.ins().iconst(ir::types::I64, 8), // Skip length field
            builder.ins().imul(
                current_count,
                builder.ins().iconst(ir::types::I64, element_size),
            ),
        );
        let insert_addr = builder.ins().iadd(grown_array, insert_offset);
        builder
            .ins()
            .store(ir::MemFlags::new(), new_element, insert_addr, 0);

        let updated_count = builder
            .ins()
            .iadd(current_count, builder.ins().iconst(ir::types::I64, 1));
        builder
            .ins()
            .jump(merge_block, &[grown_array, updated_count]);

        // Merge results
        builder.switch_to_block(merge_block);
        let final_array = builder.block_params(merge_block)[0];
        let final_count = builder.block_params(merge_block)[1];

        builder.seal_block(create_new_block);
        builder.seal_block(grow_existing_block);
        builder.seal_block(copy_loop_header);
        builder.seal_block(copy_loop_body);
        builder.seal_block(copy_loop_exit);
        builder.seal_block(merge_block);

        Ok((final_array, final_count))
    }

    /// Helper to grow a dictionary by appending a key/value entry
    fn grow_result_dict(
        &mut self,
        builder: &mut FunctionBuilder,
        current_dict: ir::Value,
        current_count: ir::Value,
        key_value: ir::Value,
        value_value: ir::Value,
    ) -> Result<(ir::Value, ir::Value)> {
        let entry_size = 16; // key + value (8 bytes each)
        let pointer_type = self.module.target_config().pointer_type();

        let create_new_block = builder.create_block();
        let grow_existing_block = builder.create_block();
        let merge_block = builder.create_block();
        builder.append_block_param(merge_block, pointer_type);
        builder.append_block_param(merge_block, ir::types::I64);

        let null_ptr = builder.ins().iconst(pointer_type, 0);
        let is_null = builder
            .ins()
            .icmp(ir::condcodes::IntCC::Equal, current_dict, null_ptr);
        builder
            .ins()
            .brif(is_null, create_new_block, &[], grow_existing_block, &[]);

        // First entry: allocate storage for one key/value pair
        builder.switch_to_block(create_new_block);
        let malloc_func_id = if let Some(&id) = self.declared_functions.get("malloc") {
            id
        } else {
            let mut sig = ir::Signature::new(self.isa.default_call_conv());
            sig.params.push(ir::AbiParam::new(ir::types::I64));
            sig.returns.push(ir::AbiParam::new(pointer_type));
            let func_id = self
                .module
                .declare_function("malloc", cranelift_module::Linkage::Import, &sig)
                .map_err(|e| anyhow!("Failed to declare malloc function: {}", e))?;
            self.declared_functions
                .insert("malloc".to_string(), func_id);
            func_id
        };

        let size_val = builder.ins().iconst(ir::types::I64, entry_size);
        let malloc_call = self
            .module
            .declare_func_in_func(malloc_func_id, builder.func);
        let new_dict = builder.ins().call(malloc_call, &[size_val]);
        builder
            .ins()
            .store(ir::MemFlags::new(), key_value, new_dict, 0);
        let value_addr = builder
            .ins()
            .iadd(new_dict, builder.ins().iconst(ir::types::I64, 8));
        builder
            .ins()
            .store(ir::MemFlags::new(), value_value, value_addr, 0);
        let single_count = builder.ins().iconst(ir::types::I64, 1);
        builder.ins().jump(merge_block, &[new_dict, single_count]);

        // Append to existing dictionary by allocating a new buffer and copying entries
        builder.switch_to_block(grow_existing_block);
        let growth = builder.ins().iconst(ir::types::I64, 4);
        let new_capacity = builder.ins().iadd(current_count, growth);
        let new_size = builder.ins().imul(
            new_capacity,
            builder.ins().iconst(ir::types::I64, entry_size as i64),
        );
        let grow_malloc_call = self
            .module
            .declare_func_in_func(malloc_func_id, builder.func);
        let grown_dict = builder.ins().call(grow_malloc_call, &[new_size]);

        // Copy existing entries one by one
        let copy_header = builder.create_block();
        let copy_body = builder.create_block();
        let copy_exit = builder.create_block();
        builder.append_block_param(copy_header, ir::types::I64);

        let initial_index = builder.ins().iconst(ir::types::I64, 0);
        builder.ins().jump(copy_header, &[initial_index]);

        builder.switch_to_block(copy_header);
        let copy_index = builder.block_params(copy_header)[0];
        let should_copy = builder.ins().icmp(
            ir::condcodes::IntCC::SignedLessThan,
            copy_index,
            current_count,
        );
        builder
            .ins()
            .brif(should_copy, copy_body, &[], copy_exit, &[]);

        builder.switch_to_block(copy_body);
        let entry_offset = builder.ins().imul(
            copy_index,
            builder.ins().iconst(ir::types::I64, entry_size as i64),
        );
        let src_addr = builder.ins().iadd(current_dict, entry_offset);
        let src_value_addr = builder
            .ins()
            .iadd(src_addr, builder.ins().iconst(ir::types::I64, 8));
        let key_loaded = builder
            .ins()
            .load(ir::types::I64, ir::MemFlags::new(), src_addr, 0);
        let value_loaded =
            builder
                .ins()
                .load(ir::types::I64, ir::MemFlags::new(), src_value_addr, 0);

        let dst_addr = builder.ins().iadd(grown_dict, entry_offset);
        let dst_value_addr = builder
            .ins()
            .iadd(dst_addr, builder.ins().iconst(ir::types::I64, 8));
        builder
            .ins()
            .store(ir::MemFlags::new(), key_loaded, dst_addr, 0);
        builder
            .ins()
            .store(ir::MemFlags::new(), value_loaded, dst_value_addr, 0);

        let next_copy_index = builder
            .ins()
            .iadd(copy_index, builder.ins().iconst(ir::types::I64, 1));
        builder.ins().jump(copy_header, &[next_copy_index]);

        builder.switch_to_block(copy_exit);
        let insert_offset = builder.ins().imul(
            current_count,
            builder.ins().iconst(ir::types::I64, entry_size as i64),
        );
        let insert_addr = builder.ins().iadd(grown_dict, insert_offset);
        let insert_value_addr = builder
            .ins()
            .iadd(insert_addr, builder.ins().iconst(ir::types::I64, 8));
        builder
            .ins()
            .store(ir::MemFlags::new(), key_value, insert_addr, 0);
        builder
            .ins()
            .store(ir::MemFlags::new(), value_value, insert_value_addr, 0);
        let updated_count = builder
            .ins()
            .iadd(current_count, builder.ins().iconst(ir::types::I64, 1));
        builder
            .ins()
            .jump(merge_block, &[grown_dict, updated_count]);

        builder.switch_to_block(merge_block);
        let merged_dict = builder.block_params(merge_block)[0];
        let merged_count = builder.block_params(merge_block)[1];

        builder.seal_block(create_new_block);
        builder.seal_block(grow_existing_block);
        builder.seal_block(copy_header);
        builder.seal_block(copy_body);
        builder.seal_block(copy_exit);
        builder.seal_block(merge_block);

        Ok((merged_dict, merged_count))
    }

    /// Lower pattern matching logic
    fn lower_pattern_match(
        &mut self,
        builder: &mut FunctionBuilder,
        pattern: &ast::nodes::Pattern,
        value: ir::Value,
        variables: &mut HashMap<String, ir::Value>,
    ) -> Result<ir::Value> {
        match pattern {
            ast::nodes::Pattern::Wildcard => {
                // Wildcard always matches
                Ok(builder.ins().iconst(ir::types::I8, 1))
            }
            ast::nodes::Pattern::Literal(lit) => {
                // Compare value with literal
                let lit_val = self.lower_literal_with_builder(builder, lit)?;
                let cmp = builder
                    .ins()
                    .icmp(ir::condcodes::IntCC::Equal, value, lit_val);
                Ok(cmp)
            }
            ast::nodes::Pattern::Identifier(name) => {
                // Bind variable and always match
                variables.insert(name.clone(), value);
                Ok(builder.ins().iconst(ir::types::I8, 1))
            }
            ast::nodes::Pattern::EnumVariant { .. } => {
                // TODO: Implement enum variant pattern matching
                // For now, assume it doesn't match
                Ok(builder.ins().iconst(ir::types::I8, 0))
            }
            ast::nodes::Pattern::Struct { .. } => {
                // TODO: Implement struct pattern matching
                // For now, assume it doesn't match
                Ok(builder.ins().iconst(ir::types::I8, 0))
            }
            ast::nodes::Pattern::Array { .. } => {
                // Safely load array length (null pointer means empty array)
                let load_length_block = builder.create_block();
                let length_merge_block = builder.create_block();
                builder.append_block_param(length_merge_block, ir::types::I64);

                let null_ptr = builder.ins().iconst(ir::types::I64, 0);
                let is_null = builder
                    .ins()
                    .icmp(ir::condcodes::IntCC::Equal, value, null_ptr);
                builder.ins().brif(
                    is_null,
                    length_merge_block,
                    &[null_ptr],
                    load_length_block,
                    &[],
                );

                builder.switch_to_block(load_length_block);
                let loaded_length =
                    builder
                        .ins()
                        .load(ir::types::I64, ir::MemFlags::new(), value, 0);
                builder.ins().jump(length_merge_block, &[loaded_length]);

                builder.switch_to_block(length_merge_block);
                let array_length = builder.block_params(length_merge_block)[0];
                let required_len = builder.ins().iconst(ir::types::I64, patterns.len() as i64);
                let length_ok = if rest.is_some() {
                    builder.ins().icmp(
                        ir::condcodes::IntCC::SignedGreaterThanOrEqual,
                        array_length,
                        required_len,
                    )
                } else {
                    builder
                        .ins()
                        .icmp(ir::condcodes::IntCC::Equal, array_length, required_len)
                };

                let fail_block = builder.create_block();
                let match_block = builder.create_block();
                let merge_block = builder.create_block();
                builder.append_block_param(merge_block, ir::types::I8);

                builder
                    .ins()
                    .brif(length_ok, match_block, &[], fail_block, &[]);

                builder.switch_to_block(fail_block);
                let fail_val = builder.ins().iconst(ir::types::I8, 0);
                builder.ins().jump(merge_block, &[fail_val]);

                builder.switch_to_block(match_block);
                if patterns.is_empty() {
                    if let Some(rest_name) = rest {
                        variables.insert(rest_name.clone(), value);
                    }
                    let success = builder.ins().iconst(ir::types::I8, 1);
                    builder.ins().jump(merge_block, &[success]);
                    builder.seal_block(match_block);
                } else {
                    let zero_i8 = builder.ins().iconst(ir::types::I8, 0);
                    let mut current_block = match_block;

                    for (idx, element_pattern) in patterns.iter().enumerate() {
                        let next_block = builder.create_block();

                        // Elements start after the length field (8 bytes each)
                        let element_offset = 8 + (idx as i64) * 8;
                        let element_addr = builder
                            .ins()
                            .iadd(value, builder.ins().iconst(ir::types::I64, element_offset));
                        let element_val = builder.ins().load(
                            ir::types::I64,
                            ir::MemFlags::new(),
                            element_addr,
                            0,
                        );

                        let element_match = self.lower_pattern_match(
                            builder,
                            element_pattern,
                            element_val,
                            variables,
                        )?;
                        let element_passes = builder.ins().icmp(
                            ir::condcodes::IntCC::NotEqual,
                            element_match,
                            zero_i8,
                        );
                        builder
                            .ins()
                            .brif(element_passes, next_block, &[], fail_block, &[]);

                        builder.seal_block(current_block);
                        builder.switch_to_block(next_block);
                        current_block = next_block;
                    }

                    if let Some(rest_name) = rest {
                        variables.insert(rest_name.clone(), value);
                    }

                    let success = builder.ins().iconst(ir::types::I8, 1);
                    builder.ins().jump(merge_block, &[success]);
                    builder.seal_block(current_block);
                }

                builder.seal_block(fail_block);
                builder.seal_block(merge_block);
                builder.seal_block(length_merge_block);
                builder.seal_block(load_length_block);

                builder.switch_to_block(merge_block);
                let result = builder.block_params(merge_block)[0];
                Ok(result)
            }
        }
    }

    /// Lower a binary operation (static version)
    fn lower_binary_op_with_builder(
        builder: &mut FunctionBuilder,
        op: BinaryOp,
        left: ir::Value,
        right: ir::Value,
    ) -> Result<ir::Value> {
        match op {
            BinaryOp::Add => Ok(builder.ins().iadd(left, right)),
            BinaryOp::Sub => Ok(builder.ins().isub(left, right)),
            BinaryOp::Mul => Ok(builder.ins().imul(left, right)),
            BinaryOp::Div => Ok(builder.ins().sdiv(left, right)),
            BinaryOp::Eq => {
                let cmp = builder.ins().icmp(ir::condcodes::IntCC::Equal, left, right);
                Ok(cmp)
            }
            BinaryOp::Ne => {
                let cmp = builder
                    .ins()
                    .icmp(ir::condcodes::IntCC::NotEqual, left, right);
                Ok(cmp)
            }
            BinaryOp::Lt => {
                let cmp = builder
                    .ins()
                    .icmp(ir::condcodes::IntCC::SignedLessThan, left, right);
                Ok(cmp)
            }
            BinaryOp::LtEq => {
                let cmp =
                    builder
                        .ins()
                        .icmp(ir::condcodes::IntCC::SignedLessThanOrEqual, left, right);
                Ok(cmp)
            }
            BinaryOp::Gt => {
                let cmp = builder
                    .ins()
                    .icmp(ir::condcodes::IntCC::SignedGreaterThan, left, right);
                Ok(cmp)
            }
            BinaryOp::GtEq => {
                let cmp =
                    builder
                        .ins()
                        .icmp(ir::condcodes::IntCC::SignedGreaterThanOrEqual, left, right);
                Ok(cmp)
            }
            BinaryOp::Is => {
                // Identity comparison - for now treat as equality
                let cmp = builder.ins().icmp(ir::condcodes::IntCC::Equal, left, right);
                Ok(cmp)
            }
            BinaryOp::IsNot => {
                // Identity not-equal - for now treat as inequality
                let cmp = builder
                    .ins()
                    .icmp(ir::condcodes::IntCC::NotEqual, left, right);
                Ok(cmp)
            }
            BinaryOp::And => Ok(builder.ins().band(left, right)), // Bitwise AND
            BinaryOp::Or => Ok(builder.ins().bor(left, right)),   // Bitwise OR
            BinaryOp::Mod => Ok(builder.ins().srem(left, right)),
        }
    }

    /// Lower a unary operation (static version)
    fn lower_unary_op_with_builder(
        builder: &mut FunctionBuilder,
        op: UnaryOp,
        val: ir::Value,
    ) -> Result<ir::Value> {
        match op {
            UnaryOp::Neg => Ok(builder.ins().ineg(val)),
            UnaryOp::Not => {
                // Logical NOT: compare with 0
                let zero = builder.ins().iconst(ir::types::I8, 0);
                Ok(builder.ins().icmp(ir::condcodes::IntCC::Equal, val, zero))
            }
        }
    }

    /// Lower a function call
    fn lower_call_with_builder(
        &mut self,
        builder: &mut FunctionBuilder,
        func: &Expr,
        args: &[Expr],
        variables: &HashMap<String, ir::Value>,
    ) -> Result<ir::Value> {
        match func {
            Expr::Identifier { name, .. } => {
                // Lower arguments
                let mut arg_values = Vec::new();
                for arg in args {
                    let arg_val = self.lower_expr_with_builder(builder, arg, variables)?;
                    arg_values.push(arg_val);
                }

                // For now, just handle built-in functions
                match name.as_str() {
                    "print" => {
                        // Declare external print function if not already declared
                        let print_func_id = if let Some(&id) = self.declared_functions.get("print")
                        {
                            id
                        } else {
                            // Declare print function: fn(*const u8) -> ()
                            let mut sig = ir::Signature::new(self.isa.default_call_conv());
                            sig.params.push(ir::AbiParam::new(
                                self.module.target_config().pointer_type(),
                            ));
                            sig.returns.push(ir::AbiParam::new(ir::types::INVALID)); // void return

                            let func_id = self
                                .module
                                .declare_function("print", cranelift_module::Linkage::Import, &sig)
                                .map_err(|e| anyhow!("Failed to declare print function: {}", e))?;

                            self.declared_functions.insert("print".to_string(), func_id);
                            func_id
                        };

                        // Call the print function
                        let call = self
                            .module
                            .declare_func_in_func(print_func_id, builder.func);
                        builder.ins().call(call, &arg_values);

                        // Print returns void, so return a dummy value
                        Ok(builder.ins().iconst(ir::types::I64, 0))
                    }
                    _ => bail!("Unknown function: {}", name),
                }
            }
            _ => bail!("Function call on non-identifier not yet supported"),
        }
    }

    /// Lower an if expression
    fn lower_if_expr_with_builder(
        &mut self,
        builder: &mut FunctionBuilder,
        condition: &Expr,
        then_branch: &Expr,
        else_branch: &Option<Box<Expr>>,
        variables: &HashMap<String, ir::Value>,
    ) -> Result<ir::Value> {
        // Evaluate condition
        let cond_val = self.lower_expr_with_builder(builder, condition, variables)?;

        // Create blocks for then, else, and merge
        let then_block = builder.create_block();
        let else_block = builder.create_block();
        let merge_block = builder.create_block();

        // Create a result variable in the merge block
        let result_type = ir::types::I64; // Default type for now
        builder.append_block_param(merge_block, result_type);

        // Branch based on condition
        builder
            .ins()
            .brif(cond_val, then_block, &[], else_block, &[]);

        // Then branch
        builder.switch_to_block(then_block);
        let then_val = self.lower_expr_with_builder(builder, then_branch, variables)?;
        builder.ins().jump(merge_block, &[then_val]);

        // Else branch
        builder.switch_to_block(else_block);
        let else_val = if let Some(else_expr) = else_branch {
            self.lower_expr_with_builder(builder, else_expr, variables)?
        } else {
            // Default else value
            builder.ins().iconst(ir::types::I64, 0)
        };
        builder.ins().jump(merge_block, &[else_val]);

        // Merge block
        builder.switch_to_block(merge_block);
        let result = builder.block_params(merge_block)[0];

        Ok(result)
    }
}

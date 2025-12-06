#![expect(
    clippy::print_stdout,
    reason = "Printing to stdout is acceptable in tests"
)]

use otterc_ffi::{DependencyConfig, PublicItem, RustTypeRef, extract_crate_spec};

#[test]
#[ignore]
fn test_deep_field_extraction() {
    let test_crate_dir = std::env::temp_dir().join("test_ffi_field_extraction");
    std::fs::create_dir_all(&test_crate_dir).unwrap();

    let manifest = "[package]
name = \"test_field_extraction\"
version = \"0.1.0\"
edition = \"2021\"
";

    let lib_rs = "
pub struct Person {
    pub name: String,
    pub age: u32,
    pub email: String,
}

pub struct Point3D {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

pub struct TupleStruct(pub i32, pub String, pub bool);

pub struct GenericContainer<T> {
    pub value: T,
    pub count: usize,
}

pub trait Drawable {
    fn draw(&self);
    fn area(&self) -> f64;
}

pub trait Resizable {
    fn resize(&mut self, scale: f64);
}
";

    std::fs::write(test_crate_dir.join("Cargo.toml"), manifest).unwrap();
    std::fs::create_dir_all(test_crate_dir.join("src")).unwrap();
    std::fs::write(test_crate_dir.join("src/lib.rs"), lib_rs).unwrap();

    let dep = DependencyConfig {
        name: "test_field_extraction".to_string(),
        version: None,
        path: Some(test_crate_dir.clone()),
        features: vec![],
        default_features: true,
    };

    let spec = extract_crate_spec(&dep).expect("Should extract successfully");

    println!("\n=== Deep Field Extraction Test ===");
    println!("Total items extracted: {}", spec.items.len());

    let person_struct = spec
        .items
        .iter()
        .find(|item| matches!(item, PublicItem::Struct { name, .. } if name == "Person"));

    assert!(person_struct.is_some(), "Should find Person struct");

    if let Some(PublicItem::Struct { name, fields, .. }) = person_struct {
        println!("\nPerson struct:");
        println!("  Name: {}", name);
        println!("  Fields: {}", fields.len());

        assert_eq!(fields.len(), 3, "Person should have exactly 3 fields");

        let name_field = fields.iter().find(|f| f.name == "name");
        assert!(name_field.is_some(), "Should have 'name' field");
        if let Some(field) = name_field {
            println!("  - name: {:?}", field.ty);
            assert!(
                matches!(field.ty, RustTypeRef::String),
                "name field should be String type"
            );
            assert!(field.is_public, "name field should be public");
        }

        let age_field = fields.iter().find(|f| f.name == "age");
        assert!(age_field.is_some(), "Should have 'age' field");
        if let Some(field) = age_field {
            println!("  - age: {:?}", field.ty);
            assert!(
                matches!(field.ty, RustTypeRef::U32),
                "age field should be u32 type"
            );
            assert!(field.is_public, "age field should be public");
        }

        let email_field = fields.iter().find(|f| f.name == "email");
        assert!(email_field.is_some(), "Should have 'email' field");
        if let Some(field) = email_field {
            println!("  - email: {:?}", field.ty);
            assert!(
                matches!(field.ty, RustTypeRef::String),
                "email field should be String type"
            );
            assert!(field.is_public, "email field should be public");
        }
    }

    let point3d_struct = spec
        .items
        .iter()
        .find(|item| matches!(item, PublicItem::Struct { name, .. } if name == "Point3D"));

    assert!(point3d_struct.is_some(), "Should find Point3D struct");

    if let Some(PublicItem::Struct { name, fields, .. }) = point3d_struct {
        println!("\nPoint3D struct:");
        println!("  Name: {}", name);
        println!("  Fields: {}", fields.len());

        assert_eq!(fields.len(), 3, "Point3D should have exactly 3 fields");

        for field in fields {
            println!("  - {}: {:?}", field.name, field.ty);
            assert!(
                matches!(field.ty, RustTypeRef::F64),
                "{} should be f64",
                field.name
            );
            assert!(field.is_public, "{} should be public", field.name);
        }
    }

    let tuple_struct = spec.items.iter().find(|item| {
        matches!(item, PublicItem::Struct { name, is_tuple, .. } if name == "TupleStruct" && *is_tuple)
    });

    assert!(tuple_struct.is_some(), "Should find TupleStruct");

    if let Some(PublicItem::Struct {
        name,
        fields,
        is_tuple,
        ..
    }) = tuple_struct
    {
        println!("\nTupleStruct:");
        println!("  Name: {}", name);
        println!("  Is tuple: {}", is_tuple);
        println!("  Fields: {}", fields.len());

        assert!(is_tuple, "TupleStruct should be marked as tuple");
        assert_eq!(fields.len(), 3, "TupleStruct should have exactly 3 fields");

        assert_eq!(fields[0].name, "0", "First field should be named '0'");
        assert!(
            matches!(fields[0].ty, RustTypeRef::I32),
            "First field should be i32"
        );

        assert_eq!(fields[1].name, "1", "Second field should be named '1'");
        assert!(
            matches!(fields[1].ty, RustTypeRef::String),
            "Second field should be String"
        );

        assert_eq!(fields[2].name, "2", "Third field should be named '2'");
        assert!(
            matches!(fields[2].ty, RustTypeRef::Bool),
            "Third field should be bool"
        );
    }

    let generic_struct = spec
        .items
        .iter()
        .find(|item| matches!(item, PublicItem::Struct { name, .. } if name == "GenericContainer"));

    assert!(
        generic_struct.is_some(),
        "Should find GenericContainer struct"
    );

    if let Some(PublicItem::Struct {
        name,
        fields,
        generics,
        ..
    }) = generic_struct
    {
        println!("\nGenericContainer struct:");
        println!("  Name: {}", name);
        println!("  Generics: {:?}", generics);
        println!("  Fields: {}", fields.len());

        assert_eq!(generics.len(), 1, "Should have 1 generic parameter");
        assert_eq!(generics[0], "T", "Generic parameter should be named 'T'");

        assert_eq!(
            fields.len(),
            2,
            "GenericContainer should have exactly 2 fields"
        );

        let value_field = fields.iter().find(|f| f.name == "value");
        assert!(value_field.is_some(), "Should have 'value' field");
        if let Some(field) = value_field {
            println!("  - value: {:?}", field.ty);
            assert!(
                matches!(&field.ty, RustTypeRef::Generic { name } if name == "T"),
                "value field should be generic type T"
            );
        }

        let count_field = fields.iter().find(|f| f.name == "count");
        assert!(count_field.is_some(), "Should have 'count' field");
        if let Some(field) = count_field {
            println!("  - count: {:?}", field.ty);
            assert!(
                matches!(field.ty, RustTypeRef::Usize),
                "count field should be usize"
            );
        }
    }

    let drawable_trait = spec
        .items
        .iter()
        .find(|item| matches!(item, PublicItem::Trait { name, .. } if name == "Drawable"));

    assert!(drawable_trait.is_some(), "Should find Drawable trait");

    if let Some(PublicItem::Trait { name, methods, .. }) = drawable_trait {
        println!("\nDrawable trait:");
        println!("  Name: {}", name);
        println!("  Methods: {}", methods.len());

        assert_eq!(methods.len(), 2, "Drawable should have exactly 2 methods");

        let draw_method = methods.iter().find(|m| m.name == "draw");
        assert!(draw_method.is_some(), "Should have 'draw' method");

        let area_method = methods.iter().find(|m| m.name == "area");
        assert!(area_method.is_some(), "Should have 'area' method");
        if let Some(method) = area_method {
            println!("  - area: {:?}", method.sig.return_type);
            assert!(
                matches!(method.sig.return_type, Some(RustTypeRef::F64)),
                "area should return f64"
            );
        }
    }

    let resizable_trait = spec
        .items
        .iter()
        .find(|item| matches!(item, PublicItem::Trait { name, .. } if name == "Resizable"));

    assert!(resizable_trait.is_some(), "Should find Resizable trait");

    if let Some(PublicItem::Trait { name, methods, .. }) = resizable_trait {
        println!("\nResizable trait:");
        println!("  Name: {}", name);
        println!("  Methods: {}", methods.len());

        assert_eq!(methods.len(), 1, "Resizable should have exactly 1 method");

        let resize_method = methods.iter().find(|m| m.name == "resize");
        assert!(resize_method.is_some(), "Should have 'resize' method");
    }

    println!("\n=== All Deep Tests Passed ===");

    let _ = std::fs::remove_dir_all(test_crate_dir);
}

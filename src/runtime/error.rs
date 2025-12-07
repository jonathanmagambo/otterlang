use std::cell::RefCell;
use std::fmt;

/// Represents a runtime error in OtterLang
#[derive(Debug, Clone)]
pub struct OtError {
    /// Error message
    pub message: String,
    /// Optional error code or type identifier
    pub code: Option<i32>,
    /// Optional additional data (could be extended for more complex error info)
    pub data: Option<String>,
}

impl OtError {
    /// Create a new error with just a message
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
            data: None,
        }
    }

    /// Create a new error with message and code
    pub fn with_code(message: impl Into<String>, code: i32) -> Self {
        Self {
            message: message.into(),
            code: Some(code),
            data: None,
        }
    }

    /// Create a new error with message, code, and data
    pub fn with_data(message: impl Into<String>, code: i32, data: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: Some(code),
            data: Some(data.into()),
        }
    }

    /// Get the error message
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Get the error code if any
    pub fn code(&self) -> Option<i32> {
        self.code
    }
}

impl fmt::Display for OtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(code) = self.code {
            write!(f, "Error {}: {}", code, self.message)
        } else {
            write!(f, "Error: {}", self.message)
        }
    }
}

impl std::error::Error for OtError {}

/// Thread-local error stack for zero-cost exception handling
/// This provides the runtime infrastructure for exception propagation
pub struct ErrorStack;

impl ErrorStack {
    thread_local! {
        static CURRENT_ERROR: RefCell<Option<OtError>> = const { RefCell::new(None) };
    }

    /// Push a new error context (for nested error handling)
    /// Returns true if an error was already pending
    pub fn push_context() -> bool {
        // For now, we only support one level of error context
        // In a more advanced implementation, this could be a stack
        Self::CURRENT_ERROR.with(|error| {
            // Don't overwrite existing errors
            error.borrow().is_some()
        })
    }

    /// Pop the error context
    /// Returns true if there was an error in this context
    pub fn pop_context() -> bool {
        // For now, this is a no-op since we only support one level
        Self::CURRENT_ERROR.with(|error| error.borrow().is_some())
    }

    /// Raise an error, setting it as the current error
    /// Returns true if there was already an error (which gets overwritten)
    pub fn raise(error: OtError) -> bool {
        Self::CURRENT_ERROR.with(|current| {
            let had_error = current.borrow().is_some();
            *current.borrow_mut() = Some(error);
            had_error
        })
    }

    /// Clear the current error
    /// Returns the error that was cleared, if any
    pub fn clear() -> Option<OtError> {
        Self::CURRENT_ERROR.with(|error| error.borrow_mut().take())
    }

    /// Get a reference to the current error without consuming it
    pub fn get_current() -> Option<std::cell::Ref<'static, OtError>> {
        // This is tricky with thread locals - we need to return a reference
        // that lives for 'static, but the RefCell borrow is scoped.
        // For now, we'll return None and implement proper error inspection later.
        // In a real implementation, we'd need to restructure this.
        None
    }

    /// Get the current error message as a string
    pub fn get_message() -> Option<String> {
        Self::CURRENT_ERROR.with(|error| error.borrow().as_ref().map(|e| e.message.clone()))
    }

    /// Check if there's currently an error
    pub fn has_error() -> bool {
        Self::CURRENT_ERROR.with(|error| error.borrow().is_some())
    }

    /// Rethrow the current error (for re-raising in handlers)
    /// This is a no-op if there's no current error
    pub fn rethrow() {
        // The error remains in the current error slot
        // This is used to re-raise exceptions in except blocks
    }
}

/// C-facing API functions for error handling
/// These functions provide a stable C ABI for LLVM-generated code
#[unsafe(no_mangle)]
pub extern "C" fn otter_error_push_context() -> bool {
    ErrorStack::push_context()
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_error_pop_context() -> bool {
    ErrorStack::pop_context()
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_error_raise(message_ptr: *const i8, message_len: usize) -> bool {
    if message_ptr.is_null() {
        return false;
    }

    // Convert C string to Rust string
    let message_bytes =
        unsafe { std::slice::from_raw_parts(message_ptr as *const u8, message_len) };
    let message = match std::str::from_utf8(message_bytes) {
        Ok(s) => s.to_string(),
        Err(_) => "Invalid UTF-8 error message".to_string(),
    };

    let error = OtError::new(message);
    ErrorStack::raise(error)
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_error_raise_with_code(
    message_ptr: *const i8,
    message_len: usize,
    code: i32,
) -> bool {
    if message_ptr.is_null() {
        return false;
    }

    let message_bytes =
        unsafe { std::slice::from_raw_parts(message_ptr as *const u8, message_len) };
    let message = match std::str::from_utf8(message_bytes) {
        Ok(s) => s.to_string(),
        Err(_) => "Invalid UTF-8 error message".to_string(),
    };

    let error = OtError::with_code(message, code);
    ErrorStack::raise(error)
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_error_clear() {
    let _ = ErrorStack::clear();
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_error_get_message(buffer: *mut i8, buffer_len: usize) -> usize {
    if let Some(message) = ErrorStack::get_message() {
        let message_bytes = message.as_bytes();
        let copy_len = std::cmp::min(message_bytes.len(), buffer_len);

        if !buffer.is_null() && copy_len > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(message_bytes.as_ptr(), buffer as *mut u8, copy_len);
            }
        }

        // Return the actual message length (may be longer than buffer)
        message_bytes.len()
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_error_has_error() -> bool {
    ErrorStack::has_error()
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_error_rethrow() {
    ErrorStack::rethrow();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let error = OtError::new("Test error");
        assert_eq!(error.message(), "Test error");
        assert_eq!(error.code(), None);

        let error_with_code = OtError::with_code("Coded error", 42);
        assert_eq!(error_with_code.message(), "Coded error");
        assert_eq!(error_with_code.code(), Some(42));
    }

    #[test]
    fn test_error_stack() {
        // Clear any existing error
        ErrorStack::clear();

        // Initially no error
        assert!(!ErrorStack::has_error());
        assert!(ErrorStack::get_message().is_none());

        // Raise an error
        let had_error = ErrorStack::raise(OtError::new("Test error"));
        assert!(!had_error); // No previous error
        assert!(ErrorStack::has_error());
        assert_eq!(ErrorStack::get_message(), Some("Test error".to_string()));

        // Clear the error
        let cleared_error = ErrorStack::clear();
        assert!(cleared_error.is_some());
        assert_eq!(cleared_error.unwrap().message(), "Test error");
        assert!(!ErrorStack::has_error());
    }

    #[test]
    fn test_error_context() {
        ErrorStack::clear();

        // Push context (no error yet)
        let had_error = ErrorStack::push_context();
        assert!(!had_error);

        // Raise error in context
        ErrorStack::raise(OtError::new("Context error"));
        assert!(ErrorStack::has_error());

        // Pop context (should indicate error was present)
        let context_had_error = ErrorStack::pop_context();
        assert!(context_had_error);
        // Error should still be present after popping
        assert!(ErrorStack::has_error());

        // Clear it
        ErrorStack::clear();
        assert!(!ErrorStack::has_error());
    }
}

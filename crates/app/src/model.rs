//! Rust handle on the C++ `VeloListModel` (`cpp/list_model.cpp`).
//!
//! Every method must run on the Qt thread (the bridge calls them from `qt_thread().queue`
//! closures). Rows travel as JSON, which keeps the FFI to plain C functions.

use std::ffi::{CString, c_char, c_int, c_void};

use serde_json::Value;

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("QtCore/QAbstractListModel");
        type QAbstractListModel;
    }
}

unsafe extern "C" {
    fn velo_model_new(roles_csv: *const c_char, parent: *mut c_void) -> *mut c_void;
    fn velo_model_replace(model: *mut c_void, json: *const c_char, len: c_int);
    fn velo_model_append(model: *mut c_void, json: *const c_char, len: c_int);
    fn velo_model_insert(model: *mut c_void, row: c_int, json: *const c_char, len: c_int);
    fn velo_model_set_field(
        model: *mut c_void,
        row: c_int,
        role: *const c_char,
        json: *const c_char,
        len: c_int,
    );
    fn velo_model_remove_row(model: *mut c_void, row: c_int);
    fn velo_model_move_row(model: *mut c_void, from: c_int, to: c_int);
}

/// One change to a list model, produced off the Qt thread and applied on it.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelOp {
    Replace(Vec<Value>),
    Append(Vec<Value>),
    /// Inserts rows before index `.0` without touching the rest (keeps scroll position).
    Insert(usize, Vec<Value>),
    SetField {
        row: usize,
        field: &'static str,
        value: Value,
    },
    Remove(usize),
    /// Moves one row; `to` is its final index.
    Move {
        from: usize,
        to: usize,
    },
}

/// Owned by its parent QObject (Qt deletes it with the parent); never freed from Rust.
pub struct ListModel(*mut c_void);

impl ListModel {
    /// `parent` is the QObject that owns the model (the singleton), or null.
    pub fn new(roles: &str, parent: *mut c_void) -> Self {
        let roles = CString::new(roles).expect("role names have no NUL");
        // SAFETY: plain C call; `roles` outlives it and the parent pointer comes from Qt.
        Self(unsafe { velo_model_new(roles.as_ptr(), parent) })
    }

    /// The model as a `QObject*`, to hand to QML inside a `QVariant`.
    pub fn as_model(&self) -> *mut ffi::QAbstractListModel {
        self.0.cast()
    }

    pub fn apply(&self, op: ModelOp) {
        match op {
            ModelOp::Replace(rows) => self.call(rows, velo_model_replace),
            ModelOp::Append(rows) => self.call(rows, velo_model_append),
            ModelOp::Insert(row, rows) => {
                let json = Value::Array(rows).to_string();
                // SAFETY: the model is live and `json` outlives the call.
                unsafe {
                    velo_model_insert(
                        self.0,
                        row as c_int,
                        json.as_ptr().cast(),
                        json.len() as c_int,
                    )
                }
            }
            ModelOp::SetField { row, field, value } => {
                let field = CString::new(field).expect("field names have no NUL");
                let json = Value::Array(vec![value]).to_string();
                // SAFETY: model pointer is live (parent-owned); strings outlive the call.
                unsafe {
                    velo_model_set_field(
                        self.0,
                        row as c_int,
                        field.as_ptr(),
                        json.as_ptr().cast(),
                        json.len() as c_int,
                    )
                }
            }
            // SAFETY: as above.
            ModelOp::Remove(row) => unsafe { velo_model_remove_row(self.0, row as c_int) },
            // SAFETY: as above.
            ModelOp::Move { from, to } => unsafe {
                velo_model_move_row(self.0, from as c_int, to as c_int)
            },
        }
    }

    fn call(&self, rows: Vec<Value>, f: unsafe extern "C" fn(*mut c_void, *const c_char, c_int)) {
        let json = Value::Array(rows).to_string();
        // SAFETY: the model is live and `json` outlives the call.
        unsafe { f(self.0, json.as_ptr().cast(), json.len() as c_int) }
    }
}

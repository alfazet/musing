use anyhow::Result;
use erased_serde::Serialize as ErasedSerialize;
use serde_json::{self, Map, Value, json};
use std::fmt::{self, Display, Formatter};

pub type JsonObject = Map<String, Value>;

// invariant: this Value is always a JsonObject
// is there a way to enforce this using the type system?
#[derive(Debug)]
pub struct Response(Value);

impl Display for Response {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", &self.0.to_string())
    }
}

impl<T> From<Result<T>> for Response {
    fn from(result: Result<T>) -> Self {
        match result {
            Ok(_) => Self::new_ok(),
            Err(e) => Self::new_err(e.to_string()),
        }
    }
}

impl Default for Response {
    fn default() -> Self {
        Self(Value::Object(JsonObject::new()))
    }
}

impl Response {
    pub fn inner(&self) -> &'_ JsonObject {
        self.0.as_object().unwrap()
    }

    pub fn inner_mut(&mut self) -> &'_ mut JsonObject {
        self.0.as_object_mut().unwrap()
    }

    pub fn new_ok() -> Self {
        Self(json!({"status": "ok"}))
    }

    pub fn new_err(reason: impl Into<String>) -> Self {
        Self(json!({"status": "err", "reason": reason.into()}))
    }

    pub fn with_item(mut self, key: impl Into<String>, value: &dyn ErasedSerialize) -> Self {
        let value = match serde_json::to_value(value) {
            Ok(value) => value,
            Err(_) => return self,
        };
        self.inner_mut().insert(key.into(), value);

        self
    }

    // returns a Response with only the keys whose values are different
    pub fn diff_with(&self, older: &Self) -> Self {
        let mut diff = JsonObject::new();
        for (key, val) in self.inner().iter() {
            let older_val = older.inner().get(key);
            if older_val.is_none() || older_val.is_some_and(|older_val| older_val != val) {
                let _ = diff.insert(key.clone(), val.clone());
            }
        }

        Self(Value::Object(diff))
    }
}

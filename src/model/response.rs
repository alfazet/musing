use anyhow::Result;
use erased_serde::Serialize as ErasedSerialize;
use serde_json::{self, Map, Value as JsonValue};
use std::fmt::{self, Display, Formatter};

#[derive(Debug)]
enum ResponseKind {
    Ok,
    Err(String),
}

#[derive(Debug)]
struct ResponseItem {
    key: String,
    value: JsonValue,
}

pub struct Response {
    kind: ResponseKind,
    items: Option<Vec<ResponseItem>>,
}

impl Display for ResponseKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let s = match self {
            ResponseKind::Ok => String::from("ok"),
            ResponseKind::Err(_) => String::from("err"),
        };

        write!(f, "{}", s)
    }
}

impl From<ResponseItem> for (String, JsonValue) {
    fn from(item: ResponseItem) -> Self {
        (item.key, item.value)
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

impl Response {
    pub fn new_ok() -> Self {
        Self {
            kind: ResponseKind::Ok,
            items: Some(Vec::new()),
        }
    }

    pub fn new_err(reason: String) -> Self {
        Self {
            kind: ResponseKind::Err(reason),
            items: None,
        }
    }

    fn append_item(&mut self, item: ResponseItem) {
        if let Some(ref mut items) = self.items {
            items.push(item);
        }
    }

    pub fn with_item(mut self, key: String, value: &dyn ErasedSerialize) -> Self {
        let value = match serde_json::to_value(value) {
            Ok(value) => value,
            Err(_) => return self,
        };
        let item = ResponseItem { key, value };
        self.append_item(item);

        self
    }

    pub fn into_json_string(self) -> Result<String> {
        let mut json_map = Map::new();
        json_map.insert("status".into(), self.kind.to_string().into());
        if let ResponseKind::Err(e) = self.kind {
            json_map.insert("reason".into(), e.to_string().into());
        }
        if let Some(items) = self.items {
            json_map.extend(items.into_iter().map(|item| item.into()));
        }

        Ok(serde_json::to_string(&json_map)?)
    }
}

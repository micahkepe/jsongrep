/*!
# JSON Schema

Defines a subset of JSON values and general definition of a JSON Schema
Object. Additionally, provides validation functions to validate JSON
instances against a schema AST.
*/
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::rc::Rc;

/// Primary JSON AST definition
#[derive(PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JSONValue {
    /// Represents a JSON object with string keys and values of any type
    Object(Box<HashMap<String, JSONValue>>),
    /// Represents a JSON array containing values of any type
    Array(Vec<JSONValue>),
    /// Represents a JSON string value
    Number(String),
    /// Represents a JSON string value
    JString(String),
    /// Represents a JSON Boolean value
    Boolean(bool),
    /// Represents a JSON null value
    Null,
}

impl JSONValue {
    /// Compute the depth of the JSON document.
    pub fn depth(&self) -> usize {
        match self {
            JSONValue::Object(map) => {
                let inner_depth = map.values().map(|v| v.depth()).max().unwrap_or(0);
                1 + inner_depth
            }
            JSONValue::Array(arr) => {
                let inner_depth = arr.iter().map(|v| v.depth()).max().unwrap_or(0);
                1 + inner_depth
            }
            JSONValue::Number(_)
            | JSONValue::JString(_)
            | JSONValue::Boolean(_)
            | JSONValue::Null => 1,
        }
    }

    /// Convert to pretty-printed JSON string
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Convert to compact JSON string
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

impl From<serde_json::Value> for JSONValue {
    fn from(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => JSONValue::Null,
            serde_json::Value::Bool(b) => JSONValue::Boolean(b),
            serde_json::Value::Number(number) => JSONValue::JString(number.to_string()),
            serde_json::Value::String(str) => JSONValue::JString(str),
            serde_json::Value::Array(values) => {
                JSONValue::Array(values.into_iter().map(JSONValue::from).collect())
            }
            serde_json::Value::Object(map) => {
                let converted_obj: HashMap<String, JSONValue> = map
                    .into_iter()
                    .map(|(k, v)| (k, JSONValue::from(v)))
                    .collect();
                JSONValue::Object(Box::new(converted_obj))
            }
        }
    }
}

// `TryFrom` over `Try` since input string may be malformed -> conversion is
// falliable
impl TryFrom<&str> for JSONValue {
    type Error = serde_json::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let serde_json_val: serde_json::Value = serde_json::from_str(value)?;
        Ok(serde_json_val.into())
    }
}

/// JSON Schema definition
#[derive(Debug, Clone)]
pub enum Schema {
    /// accepts no JSON values
    Nothing,
    /// accepts all JSON values
    Anything,
    /// accepts only the 'null' value
    Null,
    /// accepts Boolean values
    Boolean,
    /// accepts numbers
    Number,
    /// accepts strings
    String,
    /// elements of the same type
    Array(Rc<Schema>),
    /// (1) The BitMap indicates whether each property is required
    /// (2) The extra properties have the schema specified as the
    ///     3rd argument 'rest'
    /// If 'rest' is Nothing, then we specify that the object
    /// cannot have additional properties.
    Object(IndexMap<String, Rc<Schema>>, BitMap, Rc<Schema>),
    /// union (set-theoretic)
    Union(Vec<Rc<Schema>>),
    /// intersection (set-theoretic)
    Intersection(Vec<Rc<Schema>>),
}

/// A hash table where the iteration order of the key-value pairs is independent
/// of the hash values of the keys.
///
/// Simplified version from the `indexmap` crate: <https://docs.rs/indexmap/latest/indexmap/>
#[derive(Clone, Debug)]
pub struct IndexMap<K, V> {
    keys: Vec<K>,
    map: HashMap<K, V>,
}

impl<K: Eq + std::hash::Hash + Clone, V> IndexMap<K, V> {
    /// Construct a new IndexMap.
    pub fn new() -> Self {
        Self {
            keys: Vec::new(),
            map: HashMap::new(),
        }
    }

    /// Constructs an iterator over the index map.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.keys
            .iter()
            .filter_map(|key| self.map.get(key).map(|val| (key, val)))
    }

    /// Insert the given key-value pair into the index map.
    pub fn insert(&mut self, key: K, value: V) {
        if !self.keys.contains(&key) {
            self.keys.push(key.clone());
        }
        self.map.insert(key, value);
    }

    /// Retrieve the value of a given key, if it exists.
    pub fn get(&self, key: &K) -> Option<&V> {
        self.map.get(key)
    }

    /// Return whether the given key exists within the map.
    pub fn contains_key(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }
}

impl<K: Eq + std::hash::Hash + Clone, V> Default for IndexMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

/// A simplified bitmap using `Vec<bool>` as internal state representation.
#[derive(Clone, Debug)]
pub struct BitMap {
    map: Vec<bool>,
}

impl BitMap {
    /// Construct a new bitmap.
    pub fn new() -> BitMap {
        Self { map: vec![] }
    }

    /// Retrieve the value at the given index, if it exists.
    pub fn get(&self, index: usize) -> Option<bool> {
        self.map.get(index).copied()
    }

    /// Set the value at a given index.
    pub fn set(&mut self, index: usize, value: bool) {
        self.map[index] = value
    }

    /// Construct a bitmap from a bitmap of flags.
    pub fn from_required_flags(flags: Vec<bool>) -> Self {
        Self { map: flags }
    }

    /// Return whether the given index is require.
    pub fn is_required(&self, index: usize) -> bool {
        self.map.get(index).copied().unwrap_or(false)
    }
}

impl Default for BitMap {
    fn default() -> Self {
        Self::new()
    }
}

/// Validates that the given JSON data matches against the provided schema.
pub fn validate_offline(data: &JSONValue, schema: &Schema) -> bool {
    match schema {
        // Simple types
        Schema::Nothing => false, // shouldn't have received data
        Schema::Anything => true, // data is irrelevant
        Schema::Null => matches!(data, JSONValue::Null),
        Schema::Boolean => matches!(data, JSONValue::Boolean(_)),
        Schema::Number => matches!(data, JSONValue::Number(_)),
        Schema::String => matches!(data, JSONValue::JString(_)),

        // Compound types
        Schema::Array(item_sch) => {
            if let JSONValue::Array(items) = data {
                items.iter().all(|item| validate_offline(item, item_sch))
            } else {
                false
            }
        }

        Schema::Object(properties, required, rest_sch) => {
            if let JSONValue::Object(obj) = data {
                // check for required properties
                for (i, (key, sch)) in properties.iter().enumerate() {
                    let is_required = required.is_required(i);
                    match obj.get(key) {
                        Some(val) => {
                            if !validate_offline(val, sch) {
                                return false;
                            }
                        }
                        None => {
                            if is_required {
                                return false;
                            }
                        }
                    }
                }

                // check for "rest" (extra) values
                // if the schema for rest is Nothing (Schema::Nothing), the
                // value will be rejected
                for (key, val) in obj.iter() {
                    if !properties.contains_key(key) && !validate_offline(val, rest_sch) {
                        return false;
                    }
                }
                true
            } else {
                false
            }
        }

        // union -> ensure existence of at least one schema match
        Schema::Union(schemas) => schemas.iter().any(|sch| validate_offline(data, sch)),

        // intersection -> ensure input AST matches all schemas
        Schema::Intersection(schemas) => schemas.iter().all(|sch| validate_offline(data, sch)),
    }
}

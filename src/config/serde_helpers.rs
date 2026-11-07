use std::collections::HashMap;
use serde::{Deserialize, Serialize};

pub fn deserialize_string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct StringOrVecVisitor;

    impl<'de> serde::de::Visitor<'de> for StringOrVecVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string or a list of strings")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Ok(Vec::new())
            } else {
                Ok(vec![trimmed.to_string()])
            }
        }

        fn visit_seq<S>(self, mut visitor: S) -> Result<Self::Value, S::Error>
        where
            S: serde::de::SeqAccess<'de>,
        {
            let mut vec = Vec::new();
            while let Some(elem) = visitor.next_element::<String>()? {
                let trimmed = elem.trim().to_string();
                if !trimmed.is_empty() && !vec.contains(&trimmed) {
                    vec.push(trimmed);
                }
            }
            Ok(vec)
        }
    }

    deserializer.deserialize_any(StringOrVecVisitor)
}

pub fn serialize_string_or_vec<S>(vec: &Vec<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if vec.is_empty() {
        serializer.serialize_str("")
    } else if vec.len() == 1 {
        serializer.serialize_str(&vec[0])
    } else {
        vec.serialize(serializer)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StringOrVecItem {
    Single(String),
    Multiple(Vec<String>),
}

pub fn deserialize_hashmap_string_or_vec<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: HashMap<String, StringOrVecItem> = HashMap::deserialize(deserializer)?;
    let mut result = HashMap::new();
    for (k, v) in raw {
        match v {
            StringOrVecItem::Single(s) => {
                let trimmed = s.trim().to_string();
                if !trimmed.is_empty() {
                    result.insert(k, vec![trimmed]);
                }
            }
            StringOrVecItem::Multiple(list) => {
                let cleaned: Vec<String> = list
                    .into_iter()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if !cleaned.is_empty() {
                    result.insert(k, cleaned);
                }
            }
        }
    }
    Ok(result)
}

pub fn serialize_sorted_map<S, K, V>(map: &HashMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    K: Serialize + Ord,
    V: Serialize,
{
    use std::collections::BTreeMap;
    let sorted: BTreeMap<_, _> = map.iter().collect();
    sorted.serialize(serializer)
}

pub fn serialize_hashmap_string_or_vec<S>(
    map: &HashMap<String, Vec<String>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    use std::collections::BTreeMap;
    let sorted: BTreeMap<_, _> = map.iter().collect();
    let mut ser_map = serializer.serialize_map(Some(sorted.len()))?;
    for (k, v) in sorted {
        if v.len() == 1 {
            ser_map.serialize_entry(k, &v[0])?;
        } else {
            ser_map.serialize_entry(k, v)?;
        }
    }
    ser_map.end()
}

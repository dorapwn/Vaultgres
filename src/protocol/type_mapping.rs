use crate::catalog::{EnumTypeDef, Value};
use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

/// PostgreSQL type OIDs
///
/// Source: libpg's `pg_type.dat` / postgres_ext.h. VaultGres uses the
/// same numeric identifiers so that wire-protocol clients (psycopg2/3,
/// asyncpg, JDBC, ODBC, psql) decode `RowDescription` values correctly.
pub mod pg_types {
    pub const BOOL: i32 = 16;
    pub const BYTEA: i32 = 17;
    pub const INT8: i32 = 20; // bigint
    pub const INT2: i32 = 21; // smallint
    pub const INT4: i32 = 23; // int / integer
    pub const TEXT: i32 = 25;
    pub const FLOAT4: i32 = 700;
    pub const FLOAT8: i32 = 701;
    pub const VARCHAR: i32 = 1043;
    pub const DATE: i32 = 1082;
    pub const TIME: i32 = 1083;
    pub const TIMESTAMP: i32 = 1114;
    pub const NUMERIC: i32 = 1700;
    pub const JSON: i32 = 114;
    pub const JSONB: i32 = 3802;
    pub const INT4_RANGE: i32 = 3904;
    pub const INT8_RANGE: i32 = 3926;
    pub const NUM_RANGE: i32 = 3902;
    pub const DATE_RANGE: i32 = 3912;
    pub const TS_RANGE: i32 = 3908;
    pub const TSTZ_RANGE: i32 = 3910;
    // VaultGres extensions (not standard PG OIDs)
    pub const ENUM: i32 = 3500;
    pub const COMPOSITE: i32 = 3501;
}

/// Map VaultGres Value to PostgreSQL type OID and size
///
/// `type_size` follows the PG convention: -1 means "variable-length"
/// (cstring / text / bytea / numeric / json / etc.).
///
/// Tracked by https://github.com/neoalienson/Vaultgres/issues/20
pub fn value_to_pg_type(value: &Value) -> (i32, i16) {
    match value {
        // VaultGres stores Int as 64-bit; advertise as PG bigint (INT8).
        // Note: users writing `INTEGER` in SQL get DataType::Int, which is
        // also 64-bit on disk. The wire-protocol advertises bigint to be
        // honest about storage width; clients decoding as int8 are correct.
        // TODO: introduce DType::Int32 vs Int64 to round-trip SQL INTEGER vs BIGINT.
        Value::Int(_) => (pg_types::INT8, 8),
        Value::Float(_) => (pg_types::FLOAT8, 8),
        Value::Bool(_) => (pg_types::BOOL, 1),
        Value::Text(_) => (pg_types::TEXT, -1),
        Value::Json(_) => (pg_types::JSON, -1),
        // Array wire-format is text-mode `{1,2,3}`; the OID stays INT4 for
        // int[] elements — clients read each element using `int4_recv`.
        // We declare TEXT here as a conservative fallback until array
        // element OIDs are tracked properly.
        Value::Array(_) => (pg_types::TEXT, -1),
        Value::Date(_) => (pg_types::DATE, 4),
        Value::Time(_) => (pg_types::TIME, 8),
        Value::Timestamp(_) => (pg_types::TIMESTAMP, 8),
        Value::Decimal(_, _) => (pg_types::NUMERIC, -1),
        Value::Bytea(_) => (pg_types::BYTEA, -1),
        // Enum values come back as TEXT (the enum label) on the wire.
        // The OID here describes the *container* type; for variable-typed
        // enum values from generic Value::Enum we still report ENUM, but
        // serialize_value_with_enum_types rewrites to the label text.
        Value::Enum(_) => (pg_types::TEXT, -1),
        Value::Composite(_) => (pg_types::TEXT, -1),
        Value::Range(r) => range_to_pg_type(r),
        Value::Null => (pg_types::TEXT, -1),
    }
}

fn range_to_pg_type(r: &crate::catalog::Range) -> (i32, i16) {
    use crate::catalog::{RangeBound, Value};
    let bound_holds_int = |b: &Option<RangeBound>| -> bool {
        match b {
            Some(rb) => matches!(rb.value.as_ref(), Value::Int(_)),
            None => false,
        }
    };
    if bound_holds_int(&r.lower) || bound_holds_int(&r.upper) {
        (pg_types::INT8_RANGE, -1)
    } else {
        (pg_types::TEXT, -1)
    }
}

/// Serialize value to wire format (text representation)
pub fn serialize_value(value: &Value) -> Option<Vec<u8>> {
    match value {
        Value::Int(i) => Some(i.to_string().into_bytes()),
        Value::Float(f) => Some(f.to_string().into_bytes()),
        Value::Bool(b) => Some(if *b { b"t" } else { b"f" }.to_vec()),
        Value::Text(s) => Some(s.as_bytes().to_vec()),
        Value::Json(j) => Some(j.as_bytes().to_vec()),
        Value::Array(items) => {
            // PG array text-mode: `{val1,val2,NULL}` with element literals
            // matching their element type. Nested arrays use `{...}` inside
            // outer braces; strings/bytea are quoted; NULL elements are
            // bare "NULL".
            //
            // For now we render each element using serialize_value (which
            // produces the right text form for scalars), and join with
            // commas inside braces. Quoting is the conservative approach
            // — strings are wrapped in double quotes if they contain
            // special characters; numbers and bools are emitted raw.
            let mut parts: Vec<String> = Vec::with_capacity(items.len());
            for item in items {
                let part = match item {
                    Value::Null => "NULL".to_string(),
                    Value::Text(s) => {
                        // PG array quoting rules: surround with double quotes,
                        // escape internal backslashes and double quotes.
                        let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
                        format!("\"{}\"", escaped)
                    }
                    other => {
                        let bytes = serialize_value(other).unwrap_or_default();
                        String::from_utf8_lossy(&bytes).into_owned()
                    }
                };
                parts.push(part);
            }
            Some(format!("{{{}}}", parts.join(",")).into_bytes())
        }
        Value::Date(d) => {
            let epoch = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
            let date = epoch + Duration::days(*d as i64);
            Some(date.format("%Y-%m-%d").to_string().into_bytes())
        }
        Value::Time(t) => {
            let time = NaiveTime::from_hms_micro_opt(
                (*t / 3_600_000_000) as u32,                // hours
                ((*t % 3_600_000_000) / 60_000_000) as u32, // minutes
                ((*t % 60_000_000) / 1_000_000) as u32,     // seconds
                (*t % 1_000_000) as u32,                    // microseconds
            )
            .unwrap();
            Some(time.format("%H:%M:%S.%6f").to_string().into_bytes())
        }
        Value::Timestamp(ts) => {
            let epoch_date = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
            let epoch_time = NaiveTime::from_hms_opt(0, 0, 0).unwrap();
            let epoch = NaiveDateTime::new(epoch_date, epoch_time);
            let timestamp = epoch + Duration::microseconds(*ts);
            Some(timestamp.format("%Y-%m-%dT%H:%M:%S").to_string().into_bytes())
        }
        Value::Decimal(v, s) => {
            let scale_factor = 10_i128.pow(*s as u32);
            Some(
                format!(
                    "{}.{:0>width$}",
                    v / scale_factor,
                    (v % scale_factor).abs(),
                    width = *s as usize
                )
                .into_bytes(),
            )
        }
        Value::Bytea(b) => {
            let hex_str = b.iter().map(|byte| format!("{:02x}", byte)).collect::<String>();
            Some(format!("\\x{}", hex_str).into_bytes())
        }
        Value::Enum(e) => Some(format!("{}[{}]", e.type_name, e.index).into_bytes()),
        Value::Composite(c) => {
            let field_values: Vec<String> =
                c.fields.iter().map(|(_, v)| format!("{}", v)).collect();
            Some(format!("({})", field_values.join(", ")).into_bytes())
        }
        Value::Range(r) => Some(format!("{}", r).into_bytes()),
        Value::Null => None,
    }
}

/// Serialize value to wire format with enum labels resolved
pub fn serialize_value_with_enum_types(
    value: &Value,
    enum_types: &Arc<RwLock<HashMap<String, EnumTypeDef>>>,
) -> Option<Vec<u8>> {
    match value {
        Value::Enum(e) => {
            if let Ok(types) = enum_types.read() {
                if let Some(def) = types.get(&e.type_name) {
                    if let Some(label) = def.labels.get(e.index as usize) {
                        return Some(label.as_bytes().to_vec());
                    }
                }
            }
            Some(format!("{}[{}]", e.type_name, e.index).into_bytes())
        }
        _ => serialize_value(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Range;

    #[test]
    fn test_integer_serialization() {
        let value = Value::Int(42);
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(b"42".to_vec()));
    }

    #[test]
    fn test_negative_integer() {
        let value = Value::Int(-100);
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(b"-100".to_vec()));
    }

    #[test]
    fn test_text_serialization() {
        let value = Value::Text("hello".to_string());
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(b"hello".to_vec()));
    }

    #[test]
    fn test_empty_text() {
        let value = Value::Text("".to_string());
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(b"".to_vec()));
    }

    #[test]
    fn test_null_serialization() {
        let value = Value::Null;
        let serialized = serialize_value(&value);
        assert_eq!(serialized, None);
    }

    #[test]
    fn test_bool_serialization() {
        assert_eq!(serialize_value(&Value::Bool(true)), Some(b"t".to_vec()));
        assert_eq!(serialize_value(&Value::Bool(false)), Some(b"f".to_vec()));
    }

    #[test]
    fn test_float_serialization() {
        let value = Value::Float(std::f64::consts::PI);
        let serialized = serialize_value(&value);
        assert!(serialized.is_some());
        assert!(String::from_utf8_lossy(&serialized.unwrap()).starts_with("3.14"));
    }

    // ----- Type OID mapping (issue #20) -----

    #[test]
    fn test_type_mapping_int_is_int8() {
        let value = Value::Int(42);
        let (oid, size) = value_to_pg_type(&value);
        // VaultGres Int is 64-bit; advertise as PG bigint (OID 20).
        assert_eq!(oid, pg_types::INT8);
        assert_eq!(size, 8);
    }

    #[test]
    fn test_type_mapping_text() {
        let value = Value::Text("test".to_string());
        let (oid, size) = value_to_pg_type(&value);
        assert_eq!(oid, pg_types::TEXT);
        assert_eq!(size, -1);
    }

    #[test]
    fn test_type_mapping_bool() {
        let value = Value::Bool(true);
        let (oid, size) = value_to_pg_type(&value);
        assert_eq!(oid, pg_types::BOOL);
        assert_eq!(size, 1);
    }

    #[test]
    fn test_type_mapping_null_is_text() {
        let value = Value::Null;
        let (oid, size) = value_to_pg_type(&value);
        assert_eq!(oid, pg_types::TEXT);
        assert_eq!(size, -1);
    }

    #[test]
    fn test_type_mapping_float_is_float8() {
        let value = Value::Float(std::f64::consts::PI);
        let (oid, size) = value_to_pg_type(&value);
        assert_eq!(oid, pg_types::FLOAT8);
        assert_eq!(size, 8);
    }

    #[test]
    fn test_type_mapping_json_is_json_oid() {
        let value = Value::Json("{}".to_string());
        let (oid, size) = value_to_pg_type(&value);
        // Was: TEXT (25). Now: JSON (114) so clients parse as JSON, not text.
        assert_eq!(oid, pg_types::JSON);
        assert_eq!(oid, 114);
        assert_eq!(size, -1);
    }

    #[test]
    fn test_type_mapping_array_falls_back_to_text() {
        let value = Value::Array(vec![]);
        let (oid, size) = value_to_pg_type(&value);
        // Until array element OIDs are tracked, declare TEXT.
        assert_eq!(oid, pg_types::TEXT);
        assert_eq!(size, -1);
    }

    #[test]
    fn test_type_mapping_date_is_date_oid() {
        let value = Value::Date(Default::default());
        let (oid, _size) = value_to_pg_type(&value);
        // Was: INT4 (23) — clients parsed as integer. Now: DATE (1082).
        assert_eq!(oid, pg_types::DATE);
        assert_eq!(oid, 1082);
    }

    #[test]
    fn test_type_mapping_time_is_time_oid() {
        let value = Value::Time(Default::default());
        let (oid, _size) = value_to_pg_type(&value);
        // Was: INT8 (20). Now: TIME (1083).
        assert_eq!(oid, pg_types::TIME);
        assert_eq!(oid, 1083);
    }

    #[test]
    fn test_type_mapping_timestamp_is_timestamp_oid() {
        let value = Value::Timestamp(Default::default());
        let (oid, _size) = value_to_pg_type(&value);
        // Was: INT8 (20). Now: TIMESTAMP (1114).
        assert_eq!(oid, pg_types::TIMESTAMP);
        assert_eq!(oid, 1114);
    }

    #[test]
    fn test_type_mapping_decimal_is_numeric_oid() {
        let value = Value::Decimal(12345, 2);
        let (oid, size) = value_to_pg_type(&value);
        // Was: TEXT. Now: NUMERIC (1700).
        assert_eq!(oid, pg_types::NUMERIC);
        assert_eq!(oid, 1700);
        assert_eq!(size, -1);
    }

    #[test]
    fn test_type_mapping_bytea_is_bytea_oid() {
        let value = Value::Bytea(vec![1, 2, 3]);
        let (oid, size) = value_to_pg_type(&value);
        // Was: TEXT. Now: BYTEA (17).
        assert_eq!(oid, pg_types::BYTEA);
        assert_eq!(oid, 17);
        assert_eq!(size, -1);
    }

    #[test]
    fn test_type_mapping_enum_is_text() {
        // Enum values on the wire come back as the enum label TEXT.
        let value =
            Value::Enum(crate::catalog::EnumValue { type_name: "color".to_string(), index: 0 });
        let (oid, size) = value_to_pg_type(&value);
        // Was: bogus OID 3500. Now: TEXT (25).
        assert_eq!(oid, pg_types::TEXT);
        assert_eq!(size, -1);
    }

    #[test]
    fn test_type_mapping_composite_is_text() {
        let value = Value::Composite(crate::catalog::CompositeValue {
            type_name: "point".to_string(),
            fields: vec![],
        });
        let (oid, size) = value_to_pg_type(&value);
        // Was: bogus OID 3501. Now: TEXT (25).
        assert_eq!(oid, pg_types::TEXT);
        assert_eq!(size, -1);
    }

    #[test]
    fn test_type_mapping_int_range_is_int8range() {
        use crate::catalog::RangeBound;
        let r = Range {
            lower: Some(RangeBound { value: Box::new(Value::Int(1)), inclusive: true }),
            upper: Some(RangeBound { value: Box::new(Value::Int(10)), inclusive: false }),
        };
        let (oid, _size) = value_to_pg_type(&Value::Range(r));
        assert_eq!(oid, pg_types::INT8_RANGE);
    }

    #[test]
    fn test_type_mapping_non_int_range_is_text() {
        use crate::catalog::RangeBound;
        let r = Range {
            lower: Some(RangeBound {
                value: Box::new(Value::Text("a".to_string())),
                inclusive: true,
            }),
            upper: None,
        };
        let (oid, _size) = value_to_pg_type(&Value::Range(r));
        assert_eq!(oid, pg_types::TEXT);
    }

    // ----- Array serialization (issue #20) -----

    #[test]
    fn test_array_serialization_int() {
        let value = Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        let serialized = serialize_value(&value).unwrap();
        // Was: literally "[]". Now: PG text-mode "{1,2,3}".
        assert_eq!(serialized, b"{1,2,3}".to_vec());
    }

    #[test]
    fn test_array_serialization_text_quoting() {
        let value = Value::Array(vec![
            Value::Text("hello".to_string()),
            Value::Text("with\"quote".to_string()),
            Value::Text("with\\backslash".to_string()),
        ]);
        let serialized = serialize_value(&value).unwrap();
        // Strings are double-quoted unconditionally. The PG wire protocol
        // accepts this even though it's more quoting than strictly required
        // (PG only requires quoting when the string contains commas,
        // braces, whitespace, or backslashes/double-quotes). Clients parse
        // both forms identically.
        //
        // Element rendering: each Text element is wrapped in `"..."` with
        // embedded `"` escaped as `\"` and embedded `\` escaped as `\\`.
        // The wrapper `"` at each end is the boundary, not escaped content.
        assert_eq!(serialized, br#"{"hello","with\"quote","with\\backslash"}"#.to_vec());
    }

    #[test]
    fn test_array_serialization_null_elements() {
        let value = Value::Array(vec![Value::Int(1), Value::Null, Value::Int(3)]);
        let serialized = serialize_value(&value).unwrap();
        assert_eq!(serialized, b"{1,NULL,3}".to_vec());
    }

    #[test]
    fn test_array_serialization_bool() {
        let value = Value::Array(vec![Value::Bool(true), Value::Bool(false)]);
        let serialized = serialize_value(&value).unwrap();
        assert_eq!(serialized, b"{t,f}".to_vec());
    }

    #[test]
    fn test_empty_array_serialization() {
        let value = Value::Array(vec![]);
        let serialized = serialize_value(&value);
        // Was: literally "[]". Now: empty PG array "{}".
        assert_eq!(serialized, Some(b"{}".to_vec()));
    }

    #[test]
    fn test_json_serialization() {
        let value = Value::Json(r#"{"key": "value"}"#.to_string());
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(br#"{"key": "value"}"#.to_vec()));
    }

    #[test]
    fn test_empty_json_serialization() {
        let value = Value::Json("{}".to_string());
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(b"{}".to_vec()));
    }

    #[test]
    fn test_date_serialization() {
        let date_val = 8401; // 2023-01-01 is 8401 days from 2000-01-01
        let value = Value::Date(date_val);
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(b"2023-01-01".to_vec()));
    }

    #[test]
    fn test_time_serialization() {
        let time_val = 45045000000; // 12:30:45 as microseconds from midnight
        let value = Value::Time(time_val);
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(b"12:30:45.000000".to_vec()));
    }

    #[test]
    fn test_timestamp_serialization() {
        let timestamp_val = 725846400000000; // 2023-01-01T00:00:00 from 2000-01-01T00:00:00 UTC in microseconds
        let value = Value::Timestamp(timestamp_val);
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(b"2023-01-01T00:00:00".to_vec()));
    }

    #[test]
    fn test_decimal_serialization_positive() {
        let value = Value::Decimal(12345, 2); // Represents 123.45
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(b"123.45".to_vec()));
    }

    #[test]
    fn test_decimal_serialization_negative() {
        let value = Value::Decimal(-12345, 2); // Represents -123.45
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(b"-123.45".to_vec()));
    }

    #[test]
    fn test_decimal_serialization_zero() {
        let value = Value::Decimal(0, 2); // Represents 0.00
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(b"0.00".to_vec()));
    }

    #[test]
    fn test_decimal_serialization_different_scale() {
        let value = Value::Decimal(123, 0); // Represents 123
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(b"123.0".to_vec()));
    }

    #[test]
    fn test_bytea_serialization_empty() {
        let value = Value::Bytea(vec![]);
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(b"\\x".to_vec()));
    }

    #[test]
    fn test_bytea_serialization_normal() {
        let value = Value::Bytea(vec![0xDE, 0xAD, 0xBE, 0xEF]);
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(b"\\xdeadbeef".to_vec()));
    }

    #[test]
    fn test_bytea_serialization_with_zero() {
        let value = Value::Bytea(vec![0x00, 0x01, 0xFF]);
        let serialized = serialize_value(&value);
        assert_eq!(serialized, Some(b"\\x0001ff".to_vec()));
    }
}

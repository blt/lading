use std::{collections::HashMap, io::Write};

use arbitrary::{Arbitrary, Unstructured};
use rand::Rng;
use rmp_serde::Serializer;

use crate::payload::Error;
use serde::Serialize;

// Manual implementation of [this protobuf](https://github.com/DataDog/datadog-agent/blob/main/pkg/trace/pb/span.proto).
//
// ```
// syntax = "proto3";
//
// package pb;
//
// import "github.com/gogo/protobuf/gogoproto/gogo.proto";
//
// message Span {
//     // service is the name of the service with which this span is associated.
//     string service = 1 [(gogoproto.jsontag) = "service", (gogoproto.moretags) = "msg:\"service\""];
//     // name is the operation name of this span.
//     string name = 2 [(gogoproto.jsontag) = "name", (gogoproto.moretags) = "msg:\"name\""];
//     // resource is the resource name of this span, also sometimes called the endpoint (for web spans).
//     string resource = 3 [(gogoproto.jsontag) = "resource", (gogoproto.moretags) = "msg:\"resource\""];
//     // traceID is the ID of the trace to which this span belongs.
//     uint64 traceID = 4 [(gogoproto.jsontag) = "trace_id", (gogoproto.moretags) = "msg:\"trace_id\""];
//     // spanID is the ID of this span.
//     uint64 spanID = 5 [(gogoproto.jsontag) = "span_id", (gogoproto.moretags) = "msg:\"span_id\""];
//     // parentID is the ID of this span's parent, or zero if this span has no parent.
//     uint64 parentID = 6 [(gogoproto.jsontag) = "parent_id", (gogoproto.moretags) = "msg:\"parent_id\""];
//     // start is the number of nanoseconds between the Unix epoch and the beginning of this span.
//     int64 start = 7 [(gogoproto.jsontag) = "start", (gogoproto.moretags) = "msg:\"start\""];
//     // duration is the time length of this span in nanoseconds.
//     int64 duration = 8 [(gogoproto.jsontag) = "duration", (gogoproto.moretags) = "msg:\"duration\""];
//     // error is 1 if there is an error associated with this span, or 0 if there is not.
//     int32 error = 9 [(gogoproto.jsontag) = "error", (gogoproto.moretags) = "msg:\"error\""];
//     // meta is a mapping from tag name to tag value for string-valued tags.
//     map<string, string> meta = 10 [(gogoproto.jsontag) = "meta", (gogoproto.moretags) = "msg:\"meta\""];
//     // metrics is a mapping from tag name to tag value for numeric-valued tags.
//     map<string, double> metrics = 11 [(gogoproto.jsontag) = "metrics", (gogoproto.moretags) = "msg:\"metrics\""];
//     // type is the type of the service with which this span is associated.  Example values: web, db, lambda.
//     string type = 12 [(gogoproto.jsontag) = "type", (gogoproto.moretags) = "msg:\"type\""];
//     // meta_struct is a registry of structured "other" data used by, e.g., AppSec.
//     map<string, bytes> meta_struct = 13 [(gogoproto.jsontag) = "meta_struct,omitempty", (gogoproto.moretags) = "msg:\"meta_struct\""];
// }
// ```
//
// Note that this protobuf carries go-isms in it, documented
// [here](https://github.com/gogo/protobuf/blob/master/extensions.md#more-serialization-formats),
// although awkwardly this shunts to a [Google
// Groups](https://groups.google.com/g/gogoprotobuf/c/xmFnqAS6MIc) thread for
// further elaboration. I _think_ this is the equivalent of a serde rename to
// camel_case for all the field names and then `meta_struct`, the `jsontag`. If
// I understand correctly the `moretags` also implies that the field names are
// camel_case in msgpack.

/// TraceAgent span
#[derive(Arbitrary, serde::Serialize)]
struct Span {
    /// service is the name of the service with which this span is associated.
    service: String,
    /// name is the operation name of this span.
    name: String,
    /// resource is the resource name of this span, also sometimes called the endpoint (for web spans).
    resource: String,
    /// traceID is the ID of the trace to which this span belongs.
    trace_id: u64,
    /// spanID is the ID of this span.
    span_id: u64,
    /// parentID is the ID of this span's parent, or zero if this span has no parent.
    parent_id: u64,
    /// start is the number of nanoseconds between the Unix epoch and the beginning of this span.
    start: i64,
    /// duration is the time length of this span in nanoseconds.
    duration: i64,
    /// error is 1 if there is an error associated with this span, or 0 if there is not.
    error: i32,
    /// meta is a mapping from tag name to tag value for string-valued tags.
    meta: HashMap<String, String>,
    /// metrics is a mapping from tag name to tag value for numeric-valued tags.
    metrics: HashMap<String, f64>,
    /// type is the type of the service with which this span is associated.  Example values: web, db, lambda.
    #[serde(alias = "type")]
    kind: String,
    /// meta_struct is a registry of structured "other" data used by, e.g., AppSec.
    meta_struct: HashMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub(crate) enum Encoding {
    /// Encode TraceAgent payload in JSON format
    Json,
    /// Encode TraceAgent payload in MsgPack format
    #[default]
    MsgPack,
}

#[derive(Debug, Default, Clone, Copy)]
#[allow(clippy::module_name_repetitions)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub(crate) struct TraceAgent {
    encoding: Encoding,
}

impl TraceAgent {
    pub(crate) fn json() -> Self {
        Self {
            encoding: Encoding::Json,
        }
    }
    pub(crate) fn msg_pack() -> Self {
        Self {
            encoding: Encoding::MsgPack,
        }
    }
}

impl crate::payload::Serialize for TraceAgent {
    fn to_bytes<W, R>(&self, mut rng: R, max_bytes: usize, writer: &mut W) -> Result<(), Error>
    where
        R: Rng + Sized,
        W: Write,
    {
        let mut entropy: Vec<u8> = vec![0; max_bytes];
        rng.fill_bytes(&mut entropy);
        let unstructured = Unstructured::new(&entropy);

        let members = <Vec<Vec<Span>> as arbitrary::Arbitrary>::arbitrary_take_rest(unstructured)?;
        let low = 0;
        let mut high = members.len();

        loop {
            let encoding = match self.encoding {
                Encoding::Json => serde_json::to_vec(&members[low..high])?,
                Encoding::MsgPack => {
                    let mut buf = Vec::with_capacity(max_bytes);
                    members[low..high].serialize(&mut Serializer::new(&mut buf))?;
                    buf
                }
            };
            // NOTE because the type of Vec<Vec<Span>> this shrink isn't as
            // efficient as it could be. We want to shrink the tree present
            // here. This algorithm _does_ work perfectly if the tree is a
            // straight pipe.
            if encoding.len() > max_bytes {
                high /= 2;
            } else {
                writer.write_all(&encoding)?;
                break;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use proptest::prelude::*;
    use rand::{rngs::SmallRng, SeedableRng};

    use crate::payload::{Serialize, TraceAgent};

    // We want to be sure that the serialized size of the payload does not
    // exceed `max_bytes`.
    proptest! {
        #[test]
        fn payload_not_exceed_max_bytes_json(seed: u64, max_bytes: u16) {
            let max_bytes = max_bytes as usize;
            let rng = SmallRng::seed_from_u64(seed);
            let trace_agent = TraceAgent::json();

            let mut bytes = Vec::with_capacity(max_bytes);
            trace_agent.to_bytes(rng, max_bytes, &mut bytes).unwrap();
            debug_assert!(
                bytes.len() <= max_bytes,
                "{:?}",
                std::str::from_utf8(&bytes).unwrap()
            );
        }

        #[test]
        fn payload_not_exceed_max_bytes_msg_pack(seed: u64, max_bytes: u16) {
            let max_bytes = max_bytes as usize;
            let rng = SmallRng::seed_from_u64(seed);
            let trace_agent = TraceAgent::msg_pack();

            let mut bytes = Vec::with_capacity(max_bytes);
            trace_agent.to_bytes(rng, max_bytes, &mut bytes).unwrap();
            debug_assert!(
                bytes.len() <= max_bytes,
                "{:?}",
                std::str::from_utf8(&bytes).unwrap()
            );
        }
    }
}

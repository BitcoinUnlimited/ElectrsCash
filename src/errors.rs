#[derive(Debug, Copy, Clone)]
pub enum RpcErrorCode {
    ParseError = -32700,
    InvalidRequest = -32600,
    MethodNotFound = -32601,
    InvalidParams = -32602,
    InternalError = -32603,
    Other = -32000, /* Range -32000 to -32099 is serve defined */
}

error_chain! {
    types {
        Error, ErrorKind, ResultExt, Result;
    }

    errors {
        Connection(msg: String) {
            description("Connection error")
            display("Connection error: {}", msg)
        }

        Interrupt(sig: i32) {
            description("Interruption by external signal")
            display("Interrupted by signal {}", sig)
        }

        RpcError(code: RpcErrorCode, msg: String) {
            description("RPC error")
            display("RPC error ({} {:?}): {}", *code as i32, code, msg)
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum RpcErrorCode {
    ParseError = -32700,
    InvalidRequest = -32600,
    MethodNotFound = -32601,
    InvalidParams = -32602,
    InternalError = -32603,
    Other = -32000, /* Range -32000 to -32099 is serve defined */
    NotFound = -32004,
    Timeout = -32005,
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

        WebSocket(msg: String) {
            description("WebSocket error")
            display("WebSocket {}", msg)
        }
    }
}

pub fn rpc_invalid_request(what: String) -> ErrorKind {
    ErrorKind::RpcError(RpcErrorCode::InvalidRequest, what)
}

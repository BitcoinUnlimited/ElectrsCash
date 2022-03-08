use crate::errors::*;
use crate::wstcp::frame::{Frame, FrameDecoder, FrameEncoder};
use crate::wstcp::util::{self, WebSocketKey};
use async_std::net::TcpStream;
use bytecodec::io::{IoDecodeExt, IoEncodeExt, ReadBuf, StreamState, WriteBuf};
use bytecodec::{Decode, Encode, EncodeExt};
use httpcodec::{
    HeaderField, HttpVersion, NoBodyDecoder, NoBodyEncoder, ReasonPhrase, Request, RequestDecoder,
    Response, ResponseEncoder, StatusCode,
};
use std::future::Future;
use std::mem;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

const BUF_SIZE: usize = 4096;

#[derive(Debug)]
pub struct ProxyChannel {
    ws_stream: TcpStream,
    ws_rbuf: ReadBuf<Vec<u8>>,
    ws_wbuf: WriteBuf<Vec<u8>>,
    real_server_addr: SocketAddr,
    real_stream: Option<TcpStream>,
    real_stream_rstate: StreamState,
    real_stream_wstate: StreamState,
    handshake: Handshake,
    closing: Closing,
    pending_pong: Option<Vec<u8>>,
    pending_close: Option<Frame>,
    frame_decoder: FrameDecoder,
    frame_encoder: FrameEncoder,
}
impl ProxyChannel {
    pub fn new(ws_stream: TcpStream, real_server_addr: SocketAddr) -> Self {
        let _ = ws_stream.set_nodelay(true);
        info!("New proxy channel is created");
        ProxyChannel {
            ws_stream,
            ws_rbuf: ReadBuf::new(vec![0; BUF_SIZE]),
            ws_wbuf: WriteBuf::new(vec![0; BUF_SIZE]),
            real_server_addr,
            real_stream: None,
            real_stream_rstate: StreamState::Normal,
            real_stream_wstate: StreamState::Normal,
            handshake: Handshake::new(),
            closing: Closing::NotYet,
            pending_pong: None,
            pending_close: None,
            frame_decoder: FrameDecoder::default(),
            frame_encoder: FrameEncoder::default(),
        }
    }

    fn process_handshake(&mut self, cx: &mut Context) -> bool {
        loop {
            match mem::replace(&mut self.handshake, Handshake::Done) {
                Handshake::RecvRequest(mut decoder) => {
                    let result = decoder.decode_from_read_buf(&mut self.ws_rbuf);
                    if result.is_ok() && !decoder.is_idle() {
                        self.handshake = Handshake::RecvRequest(decoder);
                        break;
                    }
                    match result.and_then(|()| decoder.finish_decoding()) {
                        Err(e) => {
                            warn!("Malformed HTTP request: {}", e);
                            self.handshake = Handshake::response_bad_request();
                        }
                        Ok(request) => match self.handle_handshake_request(&request) {
                            Err(e) => {
                                warn!("Invalid WebSocket handshake request: {}", e);
                                self.handshake = Handshake::response_bad_request();
                            }
                            Ok(key) => {
                                debug!("WebSocket connecting to RPC {}", self.real_server_addr);
                                let future = TcpStream::connect(self.real_server_addr);
                                self.handshake =
                                    Handshake::ConnectToRealServer(Box::pin(future), key);
                            }
                        },
                    }
                }
                Handshake::ConnectToRealServer(mut f, key) => {
                    match Pin::new(&mut f).poll(cx).map_err(Error::from) {
                        Poll::Pending => {
                            self.handshake = Handshake::ConnectToRealServer(f, key);
                            break;
                        }
                        Poll::Ready(Err(e)) => {
                            warn!("Cannot connect to the real server: {}", e);
                            self.handshake = Handshake::response_unavailable();
                        }
                        Poll::Ready(Ok(stream)) => {
                            debug!("Connected to the real server");
                            let _ = stream.set_nodelay(true);
                            if let Ok(addr) = stream.local_addr() {
                                trace!("relay_addr {}", addr.to_string())
                            }
                            self.handshake = Handshake::response_accepted(&key);
                            self.real_stream = Some(stream);
                        }
                    }
                }
                Handshake::SendResponse(mut encoder, succeeded) => {
                    if let Err(e) = encoder.encode_to_write_buf(&mut self.ws_wbuf) {
                        warn!("Cannot write a handshake response: {}", e);
                        return false;
                    }
                    if encoder.is_idle() {
                        debug!("Handshake response has been written");
                        if succeeded {
                            info!("WebSocket handshake succeeded");
                            self.handshake = Handshake::Done;
                        } else {
                            return false;
                        }
                    } else {
                        self.handshake = Handshake::SendResponse(encoder, succeeded);
                    }
                    break;
                }
                Handshake::Done => {
                    break;
                }
            }
        }
        true
    }

    fn handle_handshake_request(&mut self, request: &Request<()>) -> Result<WebSocketKey> {
        if request.method().as_str() != "GET" {
            return Err(rpc_invalid_request("Not a GET request".to_string()).into());
        }
        if request.http_version() != HttpVersion::V1_1 {
            return Err(rpc_invalid_request("Unsupported HTTP version".to_string()).into());
        }

        let mut key = None;
        for field in request.header().fields() {
            let name = field.name();
            let value = field.value();
            if name.eq_ignore_ascii_case("upgrade") {
                if value != "websocket" {
                    return Err(
                        rpc_invalid_request("Invalid value for field 'name".to_string()).into(),
                    );
                }
            } else if name.eq_ignore_ascii_case("connection") {
                let mut values = value.split(',');
                if !values.any(|v| v.trim() == "Upgrade") {
                    return Err(rpc_invalid_request(
                        "Expected value 'Upgrade' not found in field 'connection'".to_string(),
                    )
                    .into());
                }
            } else if name.eq_ignore_ascii_case("sec-websocket-key") {
                key = Some(value.to_owned());
            } else if name.eq_ignore_ascii_case("sec-websocket-version") && value != "13" {
                return Err(
                    rpc_invalid_request("Websocket verison not supported".to_string()).into(),
                );
            }
        }

        if let Some(k) = key {
            Ok(WebSocketKey(k))
        } else {
            Err(rpc_invalid_request("sec-websocket-key missing".to_string()).into())
        }
    }

    fn process_relay(&mut self, cx: &mut Context) -> Result<()> {
        if let Err(e) = self.handle_real_stream(cx) {
            warn!("{}", e);
            self.starts_closing(1001, false)?;
        }
        if let Err(e) = self.handle_ws_stream() {
            warn!("{}", e);
            self.starts_closing(1002, false)?;
        }
        Ok(())
    }

    fn handle_real_stream(&mut self, cx: &mut Context) -> Result<()> {
        if let Some(stream) = self.real_stream.as_mut() {
            self.real_stream_rstate = self
                .frame_encoder
                .start_encoding_data(SyncReader::new(stream, cx))?;
            self.real_stream_wstate = self
                .frame_decoder
                .write_decoded_data(SyncWriter::new(stream, cx))?;
        }
        Ok(())
    }

    fn handle_ws_stream(&mut self) -> Result<()> {
        if self.frame_encoder.is_idle() {
            if let Some(data) = self.pending_pong.take() {
                debug!("Sends Ping frame: {:?}", data);
                self.frame_encoder.start_encoding(Frame::Pong { data })?;
            }
        }
        if self.frame_encoder.is_idle() {
            if let Some(frame) = self.pending_close.take() {
                self.frame_encoder.start_encoding(frame)?;
            }
        }

        self.frame_encoder.encode_to_write_buf(&mut self.ws_wbuf)?;
        if self.frame_encoder.is_idle() && self.closing.is_client_closed() {
            self.closing = Closing::Closed;
        }

        self.frame_decoder.decode_from_read_buf(&mut self.ws_rbuf)?;
        if self.frame_decoder.is_idle() {
            let frame = self.frame_decoder.finish_decoding()?;
            debug!("Received frame: {:?}", frame);
            self.handle_frame(frame)?;
        }
        Ok(())
    }

    fn handle_frame(&mut self, frame: Frame) -> Result<()> {
        match frame {
            Frame::ConnectionClose { code, reason } => {
                info!(
                    "Received Close frame: code={}, reason={:?}",
                    code,
                    String::from_utf8(reason)
                );
                match self.closing {
                    Closing::NotYet => {
                        self.starts_closing(code, true)?;
                    }
                    Closing::InProgress {
                        ref mut client_closed,
                    } => {
                        *client_closed = true;
                    }
                    _ => bail!("invalid closing state {:?}", self.closing),
                }
            }
            Frame::Ping { data } => {
                if self.closing.is_not_yet() {
                    self.pending_pong = Some(data);
                }
            }
            Frame::Pong { .. } | Frame::Data => {}
        }
        Ok(())
    }

    fn starts_closing(&mut self, code: u16, client_closed: bool) -> Result<()> {
        if self.closing != Closing::NotYet {
            bail!("starts_closing called on invalid closing state");
        }
        self.real_stream = None;
        self.real_stream_rstate = StreamState::Eos;
        self.real_stream_wstate = StreamState::Eos;
        self.closing = Closing::InProgress { client_closed };
        self.pending_close = Some(Frame::ConnectionClose {
            code,
            reason: Vec::new(),
        });
        Ok(())
    }

    fn is_ws_stream_eos(&self) -> bool {
        self.ws_rbuf.stream_state().is_eos() || self.ws_wbuf.stream_state().is_eos()
    }

    fn is_real_stream_eos(&self) -> bool {
        self.real_stream_rstate.is_eos() || self.real_stream_wstate.is_eos()
    }

    fn would_ws_stream_block(&self) -> bool {
        let empty_write =
            self.ws_wbuf.is_empty() && self.pending_close.is_none() && self.pending_pong.is_none();
        self.ws_rbuf.stream_state().would_block()
            && (empty_write || self.ws_wbuf.stream_state().would_block())
    }

    fn would_real_stream_block(&self) -> bool {
        self.real_stream_rstate.would_block()
            && (self.frame_decoder.is_data_empty() || self.real_stream_wstate.would_block())
    }
}
impl Future for ProxyChannel {
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.get_mut();
        loop {
            // WebSocket TCP stream I/O
            this.ws_rbuf
                .fill(SyncReader::new(&mut this.ws_stream, cx))?;
            this.ws_wbuf
                .flush(SyncWriter::new(&mut this.ws_stream, cx))?;
            if this.is_ws_stream_eos() {
                info!("TCP stream for WebSocket has been closed");
                return Poll::Ready(Ok(()));
            }

            // WebSocket handshake
            if !this.process_handshake(cx) {
                warn!("WebSocket handshake cannot be completed");
                return Poll::Ready(Ok(()));
            }
            if !this.handshake.done() {
                if this.would_ws_stream_block() {
                    return Poll::Pending;
                }
                continue;
            }

            if this.closing == Closing::Closed {
                info!("WebSocket channel has been closed normally");
                return Poll::Ready(Ok(()));
            }

            // Relay
            this.process_relay(cx)?;
            if this.is_real_stream_eos() && this.closing.is_not_yet() {
                info!("TCP stream for a real server has been closed");
                this.starts_closing(1000, false)?;
            }
            if this.would_ws_stream_block() && this.would_real_stream_block() {
                return Poll::Pending;
            }
        }
    }
}

enum Handshake {
    RecvRequest(Box<RequestDecoder<NoBodyDecoder>>),
    ConnectToRealServer(
        Pin<Box<(dyn Future<Output = async_std::io::Result<TcpStream>> + Send + 'static)>>,
        WebSocketKey,
    ),
    SendResponse(ResponseEncoder<NoBodyEncoder>, bool),
    Done,
}
impl Handshake {
    fn new() -> Self {
        Handshake::RecvRequest(Box::new(RequestDecoder::default()))
    }

    fn done(&self) -> bool {
        matches!(*self, Handshake::Done)
    }

    fn response_accepted(key: &WebSocketKey) -> Self {
        let hash = util::calc_accept_hash(key);

        unsafe {
            let mut response = Response::new(
                HttpVersion::V1_1,
                StatusCode::new_unchecked(101),
                ReasonPhrase::new_unchecked("Switching Protocols"),
                (),
            );
            response
                .header_mut()
                .add_field(HeaderField::new_unchecked("Upgrade", "websocket"))
                .add_field(HeaderField::new_unchecked("Connection", "Upgrade"))
                .add_field(HeaderField::new_unchecked("Sec-WebSocket-Accept", &hash));

            let encoder = ResponseEncoder::with_item(response).expect("Never fails");
            Handshake::SendResponse(encoder, true)
        }
    }

    fn response_bad_request() -> Self {
        unsafe {
            let mut response = Response::new(
                HttpVersion::V1_1,
                StatusCode::new_unchecked(400),
                ReasonPhrase::new_unchecked("Bad Request"),
                (),
            );
            response
                .header_mut()
                .add_field(HeaderField::new_unchecked("Content-Length", "0"));
            let encoder = ResponseEncoder::with_item(response).expect("Never fails");
            Handshake::SendResponse(encoder, false)
        }
    }

    fn response_unavailable() -> Self {
        unsafe {
            let mut response = Response::new(
                HttpVersion::V1_1,
                StatusCode::new_unchecked(503),
                ReasonPhrase::new_unchecked("Service Unavailable"),
                (),
            );
            response
                .header_mut()
                .add_field(HeaderField::new_unchecked("Content-Length", "0"));
            let encoder = ResponseEncoder::with_item(response).expect("Never fails");
            Handshake::SendResponse(encoder, false)
        }
    }
}

impl std::fmt::Debug for Handshake {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Handshake {{ .. }}")
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Closing {
    NotYet,
    InProgress { client_closed: bool },
    Closed,
}
impl Closing {
    fn is_not_yet(&self) -> bool {
        *self == Closing::NotYet
    }

    fn is_client_closed(&self) -> bool {
        *self
            == Closing::InProgress {
                client_closed: true,
            }
    }
}

#[derive(Debug)]
struct SyncReader<'a, 'b, 'c, T> {
    inner: &'a mut T,
    cx: &'b mut Context<'c>,
}

impl<'a, 'b, 'c, T: async_std::io::Read> SyncReader<'a, 'b, 'c, T> {
    fn new(inner: &'a mut T, cx: &'b mut Context<'c>) -> Self {
        Self { inner, cx }
    }
}

impl<'a, 'b, 'c, T> std::io::Read for SyncReader<'a, 'b, 'c, T>
where
    T: async_std::io::Read + std::marker::Unpin,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match Pin::new(&mut *self.inner).poll_read(self.cx, buf) {
            Poll::Pending => Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "Would block",
            )),
            Poll::Ready(result) => result,
        }
    }
}

#[derive(Debug)]
struct SyncWriter<'a, 'b, 'c, T> {
    inner: &'a mut T,
    cx: &'b mut Context<'c>,
}

impl<'a, 'b, 'c, T: async_std::io::Write> SyncWriter<'a, 'b, 'c, T> {
    fn new(inner: &'a mut T, cx: &'b mut Context<'c>) -> Self {
        Self { inner, cx }
    }
}

impl<'a, 'b, 'c, T> std::io::Write for SyncWriter<'a, 'b, 'c, T>
where
    T: async_std::io::Write + std::marker::Unpin,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match Pin::new(&mut *self.inner).poll_write(self.cx, buf) {
            Poll::Pending => Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "Would block",
            )),
            Poll::Ready(result) => result,
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match Pin::new(&mut *self.inner).poll_flush(self.cx) {
            Poll::Pending => Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "Would block",
            )),
            Poll::Ready(result) => result,
        }
    }
}

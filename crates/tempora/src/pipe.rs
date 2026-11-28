//! The local control channel between the dashboard and the recorder: one named pipe per logon
//! session, readable and writable only by that session's logon SID. Each message is a 4-byte
//! little-endian length followed by that many bytes of JSON.

use crate::{
    model::{ControlRequest, ControlResponse},
    win,
};
use std::{
    ffi::c_void,
    fs::File,
    io::{self, Read, Write},
    os::windows::io::{AsRawHandle, FromRawHandle, RawHandle},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_NO_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
        INVALID_HANDLE_VALUE, LocalFree,
    },
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        },
        GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_GROUPS, TOKEN_QUERY, TokenLogonSid,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_APPEND_DATA, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_GENERIC_READ,
        FILE_GENERIC_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION,
        SECURITY_SQOS_PRESENT,
    },
    System::{
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientSessionId,
            GetNamedPipeServerProcessId, GetNamedPipeServerSessionId, PIPE_NOWAIT,
            PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, WaitNamedPipeW,
        },
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

const CLIENT_ACCESS: u32 = FILE_GENERIC_READ | (FILE_GENERIC_WRITE & !FILE_APPEND_DATA);
const BUFFER_BYTES: u32 = 64 * 1024;
const MAX_PAYLOAD: usize = 8 * 1024 * 1024;
const LENGTH_BYTES: usize = size_of::<u32>();
const PEER_TIMEOUT: Duration = Duration::from_secs(5);

/// Why a request to the recorder failed.
#[derive(Debug, thiserror::Error)]
pub enum RequestError {
    /// No recorder is listening in this logon session.
    #[error("The recorder isn't running")]
    NotRunning,
    /// Something other than our own recorder answered.
    #[error("{0}")]
    Untrusted(&'static str),
    /// The recorder received the request and refused it.
    #[error("{0}")]
    Refused(String),
    /// The recorder answered with a reply of the wrong kind.
    #[error("Unexpected recorder reply")]
    UnexpectedReply,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The current process's Windows session.
pub fn session_id() -> io::Result<u32> {
    win::process_session(win::current_process_id()).ok_or_else(io::Error::last_os_error)
}

/// Sends one request to the recorder in the current logon session and waits for its reply.
pub fn request(request: &ControlRequest) -> Result<ControlResponse, RequestError> {
    let session = session_id()?;
    let sid = logon_sid(current_process())?;
    let name = pipe_name(session, &sid);
    request_at(request, session, &sid, &name)
}

/// Sends a request that expects a plain `Ok`.
pub fn command(command: &ControlRequest) -> Result<(), RequestError> {
    command_changed(command).map(|_| ())
}

/// Sends a request and reports whether the recorder changed anything. A correction that saved
/// nothing new answers `Unchanged`: still a success, but with no undo step to offer.
pub fn command_changed(command: &ControlRequest) -> Result<bool, RequestError> {
    changed_from_reply(request(command)?)
}

fn changed_from_reply(reply: ControlResponse) -> Result<bool, RequestError> {
    match reply {
        ControlResponse::Ok => Ok(true),
        ControlResponse::Unchanged => Ok(false),
        ControlResponse::Error(error) => Err(RequestError::Refused(error)),
        ControlResponse::Status(_) => Err(RequestError::UnexpectedReply),
    }
}

/// Asks the recorder for its status.
pub fn status() -> Result<crate::model::RecorderStatus, RequestError> {
    match request(&ControlRequest::Status)? {
        ControlResponse::Status(status) => Ok(status),
        ControlResponse::Error(error) => Err(RequestError::Refused(error)),
        ControlResponse::Ok | ControlResponse::Unchanged => Err(RequestError::UnexpectedReply),
    }
}

fn request_at(
    request: &ControlRequest,
    session: u32,
    sid: &str,
    name: &[u16],
) -> Result<ControlResponse, RequestError> {
    let open = || {
        // SAFETY: `name` is a NUL-terminated UTF-16 pipe name; the other arguments are flags and
        // null optional pointers. The handle is checked by `owned_file`.
        unsafe {
            CreateFileW(
                name.as_ptr(),
                CLIENT_ACCESS,
                0,
                null(),
                OPEN_EXISTING,
                SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                null_mut(),
            )
        }
    };
    let mut handle = open();
    // A busy pipe is serving another client; wait up to a second for it.
    if handle == INVALID_HANDLE_VALUE && last_error_is(ERROR_PIPE_BUSY) {
        // SAFETY: `name` is a NUL-terminated UTF-16 pipe name.
        if unsafe { WaitNamedPipeW(name.as_ptr(), 1_000) } != 0 {
            handle = open();
        }
    }
    if handle == INVALID_HANDLE_VALUE && last_error_is(ERROR_FILE_NOT_FOUND) {
        return Err(RequestError::NotRunning);
    }
    let mut pipe = owned_file(handle)?;
    let (mut server_session, mut server_pid) = (0, 0);
    // SAFETY: The pipe handle is live, and both out pointers are valid.
    let identified = unsafe {
        GetNamedPipeServerSessionId(pipe.as_raw_handle(), &mut server_session) != 0
            && GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut server_pid) != 0
    };
    if !identified || server_session != session {
        return Err(RequestError::Untrusted("Unexpected recorder session"));
    }
    let server = win::Process::open(server_pid).ok_or_else(io::Error::last_os_error)?;
    if logon_sid(server.as_raw_handle())? != sid {
        return Err(RequestError::Untrusted("Unexpected recorder identity"));
    }
    pipe.write_all(&encode_frame(request)?)?;
    let mut size = [0; LENGTH_BYTES];
    pipe.read_exact(&mut size)?;
    let mut payload = vec![0; payload_size(size)?];
    pipe.read_exact(&mut payload)?;
    Ok(serde_json::from_slice(&payload).map_err(io::Error::other)?)
}

fn last_error_is(code: u32) -> bool {
    io::Error::last_os_error().raw_os_error() == Some(code as i32)
}

/// The current process's pseudo-handle. It needs no closing and is valid for the process's life.
fn current_process() -> RawHandle {
    // SAFETY: No arguments; returns a constant pseudo-handle.
    unsafe { GetCurrentProcess() }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn pipe_name(session: u32, sid: &str) -> Vec<u16> {
    wide(&format!(r"\\.\pipe\Tempora-v1-{session}-{sid}"))
}

/// Takes ownership of a handle returned by a Windows call, or the call's error.
fn owned_file(handle: RawHandle) -> io::Result<File> {
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: A valid handle that nothing else owns; the File closes it once.
    Ok(unsafe { File::from_raw_handle(handle) })
}

/// Memory allocated by Windows with `LocalAlloc`, freed on drop.
struct LocalMemory(*mut c_void);

impl Drop for LocalMemory {
    fn drop(&mut self) {
        // SAFETY: The pointer came from a Windows call documented to allocate with LocalAlloc,
        // and is freed once.
        unsafe { LocalFree(self.0) };
    }
}

/// The logon SID of the process behind `process`, as a string (`S-1-5-5-…`). The pipe is
/// restricted to it, which keeps other users and other sessions of the same user out.
fn logon_sid(process: RawHandle) -> io::Result<String> {
    let mut token = null_mut();
    // SAFETY: `process` is a live process handle and `token` a valid out pointer.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = owned_file(token)?;
    let mut len = 0;
    // A first call with no buffer reports the size needed.
    // SAFETY: A null buffer with zero length is the documented size query.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenLogonSid,
            null_mut(),
            0,
            &mut len,
        )
    };
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    // Token information contains pointers and requires pointer alignment.
    let mut buffer = vec![0usize; (len as usize).div_ceil(size_of::<usize>())];
    // SAFETY: The buffer is at least `len` bytes and pointer-aligned.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenLogonSid,
            buffer.as_mut_ptr().cast(),
            len,
            &mut len,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: On success the buffer starts with a TOKEN_GROUPS, suitably aligned (see above).
    let groups = unsafe { &*buffer.as_ptr().cast::<TOKEN_GROUPS>() };
    if groups.GroupCount != 1 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Missing logon security identifier",
        ));
    }
    let mut sid = null_mut();
    // SAFETY: The SID points into `buffer`, which is alive; `sid` is a valid out pointer.
    if unsafe { ConvertSidToStringSidW(groups.Groups[0].Sid, &mut sid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let owned = LocalMemory(sid.cast());
    // SAFETY: ConvertSidToStringSidW returns a NUL-terminated UTF-16 string, live until
    // `owned` is dropped at the end of this function.
    let text = unsafe {
        let len = (0..).take_while(|&index| *sid.add(index) != 0).count();
        std::slice::from_raw_parts(sid, len)
    };
    let result = String::from_utf16(text).map_err(io::Error::other);
    drop(owned);
    result
}

fn payload_size(size: [u8; LENGTH_BYTES]) -> io::Result<usize> {
    let size = u32::from_le_bytes(size) as usize;
    if size == 0 || size > MAX_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid recorder message size",
        ));
    }
    Ok(size)
}

fn encode_frame(value: &impl serde::Serialize) -> io::Result<Vec<u8>> {
    let mut frame = vec![0; LENGTH_BYTES];
    serde_json::to_writer(&mut frame, value).map_err(io::Error::other)?;
    let size = ((frame.len() - LENGTH_BYTES) as u32).to_le_bytes();
    payload_size(size)?;
    frame[..LENGTH_BYTES].copy_from_slice(&size);
    Ok(frame)
}

#[derive(Default)]
struct IncomingRequest {
    bytes: Vec<u8>,
    payload_len: Option<usize>,
}

impl IncomingRequest {
    fn append(&mut self, chunk: &[u8]) -> io::Result<Option<ControlRequest>> {
        self.bytes.extend_from_slice(chunk);
        if self.payload_len.is_none()
            && let Some(prefix) = self.bytes.first_chunk::<LENGTH_BYTES>()
        {
            self.payload_len = Some(payload_size(*prefix)?);
        }
        let Some(len) = self.payload_len else {
            return Ok(None);
        };
        let payload = &self.bytes[LENGTH_BYTES..];
        if payload.len() < len {
            return Ok(None);
        }
        // Decode all buffered payload bytes so a second JSON value in the same read is rejected.
        serde_json::from_slice(payload)
            .map(Some)
            .map_err(io::Error::other)
    }
}

enum Exchange {
    Reading(IncomingRequest),
    Writing { bytes: Vec<u8>, sent: usize },
    AwaitingDisconnect,
}

struct Peer {
    last_progress: Instant,
    exchange: Exchange,
}

/// The recorder's end of the pipe. Non-blocking: the recorder polls it from its timer and
/// serves one client at a time.
pub struct Server {
    pipe: File,
    session: u32,
    peer: Option<Peer>,
}

impl Server {
    /// Creates the pipe for this logon session. Fails if another recorder already owns it.
    pub fn bind() -> io::Result<Self> {
        let session = session_id()?;
        let sid = logon_sid(current_process())?;
        let name = pipe_name(session, &sid);
        Self::bind_at(session, &sid, &name)
    }

    fn bind_at(session: u32, sid: &str, name: &[u16]) -> io::Result<Self> {
        // Grant byte reads and writes without granting FILE_CREATE_PIPE_INSTANCE.
        let descriptor = wide(&format!("D:P(A;;0x{CLIENT_ACCESS:x};;;{sid})"));
        let mut security_descriptor = null_mut();
        // SAFETY: `descriptor` is a NUL-terminated SDDL string (revision 1); the out pointer is
        // valid and the size pointer is optional.
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                descriptor.as_ptr(),
                1,
                &mut security_descriptor,
                null_mut(),
            )
        };
        if converted == 0 {
            return Err(io::Error::last_os_error());
        }
        let security_descriptor = LocalMemory(security_descriptor);
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: security_descriptor.0,
            bInheritHandle: 0,
        };
        // SAFETY: `name` is a NUL-terminated pipe name; `attributes` and the descriptor it points
        // to stay alive for the call. The handle is checked by `owned_file`.
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                BUFFER_BYTES,
                BUFFER_BYTES,
                1_000,
                &attributes,
            )
        };
        drop(security_descriptor);
        Ok(Self {
            pipe: owned_file(handle)?,
            session,
            peer: None,
        })
    }

    /// Advances the current exchange, returning a request once one has fully arrived.
    pub fn poll(&mut self) -> Option<ControlRequest> {
        if self.peer.is_none() {
            // In non-blocking mode a connected client reports ERROR_PIPE_CONNECTED; success
            // means no client yet.
            // SAFETY: The pipe handle is live; no OVERLAPPED is used.
            if unsafe { ConnectNamedPipe(self.pipe.as_raw_handle(), null_mut()) } != 0 {
                return None;
            }
            let error = io::Error::last_os_error().raw_os_error();
            if error == Some(ERROR_NO_DATA as i32) {
                // A client connected and left before we saw it.
                self.disconnect();
            }
            if error != Some(ERROR_PIPE_CONNECTED as i32) {
                return None;
            }
            let mut session = 0;
            // SAFETY: The pipe handle is live and `session` a valid out pointer.
            let identified =
                unsafe { GetNamedPipeClientSessionId(self.pipe.as_raw_handle(), &mut session) }
                    != 0;
            if !identified || session != self.session {
                self.disconnect();
                return None;
            }
            self.peer = Some(Peer {
                last_progress: Instant::now(),
                exchange: Exchange::Reading(IncomingRequest::default()),
            });
        }

        match self.advance() {
            Ok(request) => request,
            Err(_) => {
                self.disconnect();
                self.peer = None;
                None
            }
        }
    }

    /// Drops the current client so the next one can connect.
    fn disconnect(&self) {
        // SAFETY: The pipe handle is live.
        unsafe { DisconnectNamedPipe(self.pipe.as_raw_handle()) };
    }

    fn advance(&mut self) -> io::Result<Option<ControlRequest>> {
        let peer = self
            .peer
            .as_mut()
            .expect("poll establishes a peer before advancing it");
        if peer.last_progress.elapsed() > PEER_TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Recorder control request timed out",
            ));
        }
        match &mut peer.exchange {
            Exchange::Reading(request) => {
                let mut chunk = [0; BUFFER_BYTES as usize];
                let count = read_available(&mut self.pipe, &mut chunk)?.unwrap_or(0);
                if count > 0 {
                    peer.last_progress = Instant::now();
                }
                return request.append(&chunk[..count]);
            }
            Exchange::Writing { bytes, sent } => {
                let written = self.pipe.write(&bytes[*sent..])?;
                *sent += written;
                if written > 0 {
                    peer.last_progress = Instant::now();
                }
                if *sent == bytes.len() {
                    peer.exchange = Exchange::AwaitingDisconnect;
                }
            }
            Exchange::AwaitingDisconnect => {
                // The peer closes after reading. Disconnecting early discards unread output.
                if read_available(&mut self.pipe, &mut [0; 1])?.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Unexpected extra recorder data",
                    ));
                }
            }
        }
        Ok(None)
    }

    /// Queues the reply to the request `poll` returned; later polls send it.
    pub fn respond(&mut self, response: &ControlResponse) {
        if let Some(peer) = &mut self.peer {
            let bytes = encode_frame(response).unwrap_or_else(|_| {
                encode_frame(&ControlResponse::Error(
                    "Recorder response is too large".into(),
                ))
                .expect("the fixed error response fits within the frame size limit")
            });
            peer.last_progress = Instant::now();
            peer.exchange = Exchange::Writing { bytes, sent: 0 };
        }
    }

    /// Whether no client is connected.
    pub fn idle(&self) -> bool {
        self.peer.is_none()
    }
}

fn read_available(pipe: &mut File, buffer: &mut [u8]) -> io::Result<Option<usize>> {
    match pipe.read(buffer) {
        Ok(0) => Err(io::ErrorKind::UnexpectedEof.into()),
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.raw_os_error() == Some(ERROR_NO_DATA as i32) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_a_split_request_prefix_and_payload() -> io::Result<()> {
        let frame = encode_frame(&ControlRequest::SetPaused(true))?;
        let mut incoming = IncomingRequest::default();
        assert!(incoming.append(&frame[..2])?.is_none());
        assert!(incoming.append(&frame[2..frame.len() - 1])?.is_none());
        assert!(matches!(
            incoming.append(&frame[frame.len() - 1..])?,
            Some(ControlRequest::SetPaused(true))
        ));
        Ok(())
    }

    #[test]
    fn rejects_an_empty_frame_length() {
        let error = IncomingRequest::default()
            .append(&[0; LENGTH_BYTES])
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "Invalid recorder message size");
    }

    #[test]
    fn an_unchanged_reply_is_a_success_that_changed_nothing() {
        assert!(matches!(changed_from_reply(ControlResponse::Ok), Ok(true)));
        assert!(matches!(
            changed_from_reply(ControlResponse::Unchanged),
            Ok(false)
        ));
        assert!(matches!(
            changed_from_reply(ControlResponse::Error("Refused".into())),
            Err(RequestError::Refused(message)) if message == "Refused"
        ));
    }

    #[test]
    #[ignore = "needs an interactive desktop session"]
    fn exchanges_a_response_on_a_reused_session_pipe() {
        let session = session_id().unwrap();
        let sid = logon_sid(current_process()).unwrap();
        let name = pipe_name(
            session,
            &format!("{sid}-test-{}", win::current_process_id()),
        );
        let mut server = Server::bind_at(session, &sid, &name).unwrap();
        assert!(Server::bind_at(session, &sid, &name).is_err());

        for payload in ["ready".to_owned(), "x".repeat(BUFFER_BYTES as usize * 2)] {
            let peer = {
                let sid = sid.clone();
                let name = name.clone();
                std::thread::spawn(move || {
                    request_at(&ControlRequest::Status, session, &sid, &name)
                })
            };
            let started = Instant::now();
            while !peer.is_finished() || !server.idle() {
                assert!(
                    started.elapsed() < Duration::from_secs(3),
                    "Control exchange stalled"
                );
                if let Some(ControlRequest::Status) = server.poll() {
                    server.respond(&ControlResponse::Error(payload.clone()));
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(matches!(
                peer.join().unwrap().unwrap(),
                ControlResponse::Error(received) if received == payload
            ));
        }
    }
}

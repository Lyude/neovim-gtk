mod client;
mod handler;
mod redraw_handler;
mod ext;

pub use self::redraw_handler::{CompleteItem, NvimCommand, RedrawMode};
pub use self::client::NeovimClient;
pub use self::ext::*;
pub use self::handler::NvimHandler;

use std::{
    error, fmt, env, result,
    convert::TryFrom,
    num::*,
    time::Duration,
    pin::Pin,
    process::Stdio,
    task::{Context, Poll},
    ops::{Deref, DerefMut},
    future::Future,
    sync::Arc,
};

use tokio::{
    io::{self, AsyncWrite},
    process::{Command, ChildStdin},
    time::{timeout, error::Elapsed},
    task::JoinHandle,
    runtime::Runtime,
};
use tokio_util::compat::*;

use futures::future::{
    FutureExt, BoxFuture,
};

use nvim_rs::{
    self,
    UiAttachOptions, Value,
    error::{LoopError, CallError, DecodeError},
    compat::tokio::Compat,
};

use crate::nvim_config::NvimConfig;

#[derive(Debug)]
pub struct NvimInitError {
    source: Box<dyn error::Error>,
    cmd: Option<String>,
}

impl NvimInitError {
    pub fn new_post_init<E>(error: E) -> NvimInitError
    where
        E: Into<Box<dyn error::Error>>,
    {
        NvimInitError {
            cmd: None,
            source: error.into(),
        }
    }

    pub fn new<E>(cmd: &Command, error: E) -> NvimInitError
    where
        E: Into<Box<dyn error::Error>>,
    {
        NvimInitError {
            cmd: Some(format!("{:?}", cmd)),
            source: error.into(),
        }
    }

    pub fn source(&self) -> String {
        format!("{}", self.source)
    }

    pub fn cmd(&self) -> Option<&String> {
        self.cmd.as_ref()
    }
}

impl fmt::Display for NvimInitError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.source)
    }
}

impl error::Error for NvimInitError {
    fn description(&self) -> &str {
        "Can't start nvim instance"
    }

    fn cause(&self) -> Option<&dyn error::Error> {
        Some(&*self.source)
    }
}

#[derive(Debug)]
pub enum SessionError {
    CallError(Box<CallError>),
    TimeoutError(Elapsed),
}

impl error::Error for SessionError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::CallError(e) => Some(e),
            Self::TimeoutError(e) => Some(e),
        }
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::CallError(e) => write!(f, "{:?}", e),
            Self::TimeoutError(e) => write!(f, "{:?}", e),
        }
    }
}

impl From<Box<CallError>> for SessionError {
    fn from(err: Box<CallError>) -> Self {
        SessionError::CallError(err)
    }
}

impl From<Elapsed> for SessionError {
    fn from(err: Elapsed) -> Self {
        SessionError::TimeoutError(err)
    }
}

#[cfg(target_os = "windows")]
fn set_windows_creation_flags(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
}

pub enum NvimWriter {
    ChildProcess(ChildStdin),
}

impl From<ChildStdin> for NvimWriter {
    fn from(stdin: ChildStdin) -> Self {
        Self::ChildProcess(stdin)
    }
}

impl AsyncWrite for NvimWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8]
    ) -> Poll<io::Result<usize>> {
        match *self {
            Self::ChildProcess(ref mut stdin) => Pin::new(stdin).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match *self {
            Self::ChildProcess(ref mut stdin) => Pin::new(stdin).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match *self {
            Self::ChildProcess(ref mut stdin) => Pin::new(stdin).poll_shutdown(cx),
        }
    }
}

pub type Neovim = nvim_rs::Neovim<Compat<NvimWriter>>;
pub type Tabpage = nvim_rs::Tabpage<Compat<NvimWriter>>;

/// Our main wrapper for `Neovim`, which also provides access to the timeout duration for this
/// session
#[derive(Clone)]
pub struct NvimSession {
    nvim: Neovim,
    timeout: Duration,
    runtime: Arc<Runtime>,
}

impl NvimSession {
    pub fn new_child<'a>(
        mut cmd: Command,
        handler: NvimHandler,
        timeout: Duration,
    ) -> Result<(NvimSession, BoxFuture<'a, Result<(), Box<LoopError>>>), NvimInitError> {
        let runtime = Arc::new(
            Runtime::new().map_err(|e| NvimInitError::new(&cmd, e))?
        );
        let mut child = runtime.block_on(async move {
            cmd.spawn().map_err(|e| NvimInitError::new(&cmd, e))
        })?;

        let (nvim, io_future) = Neovim::new(
            child.stdout.take().unwrap().compat(),
            NvimWriter::from(child.stdin.take().unwrap()).compat_write(),
            handler
        );

        Ok((Self { nvim, timeout, runtime }, io_future.boxed()))
    }

    /// Wrap a future from an RPC call to neovim within a timeout
    pub async fn timeout<F, T>(&self, f: F) -> Result<T, SessionError>
    where
        F: Future<Output = Result<T, Box<CallError>>>,
    {
        match timeout(self.timeout, f).await {
            Ok(f) => match f {
                Ok(f) => Ok(f),
                Err(e) => Err(e.into()),
            },
            Err(e) => Err(e.into()),
        }
    }

    /// Execute a future on the current thread using this session's tokio runtime
    #[inline]
    pub fn block_on<T>(&self, f: impl Future<Output = T>) -> T {
        self.runtime.block_on(f)
    }

    /// Spawn a future on this session's tokio runtime
    #[inline]
    pub fn spawn(&self, f: impl Future<Output = ()> + Send + 'static) -> JoinHandle<()> {
        self.runtime.spawn(f)
    }

    /// Wrap a future from an RPC call to neovim inside a timeout, and execute it on the current
    /// thread using this session's tokio runtime
    pub fn block_timeout<F, T>(&self, f: F) -> Result<T, SessionError>
    where
        F: Future<Output = Result<T, Box<CallError>>>,
    {
        self.block_on(self.timeout(f))
    }

    #[doc(hidden)]
    pub fn spawn_timeout<F, T>(&self, f: F) -> JoinHandle<()>
    where
        F: Future<Output = Result<T, Box<CallError>>> + Send + 'static
    {
        let nvim = self.clone();

        self.spawn(async move {
            nvim.timeout(f).await.report_err()
        })
    }

    #[doc(hidden)]
    pub fn spawn_timeout_user_err<F, T>(&self, f: F) -> JoinHandle<()>
    where
        F: Future<Output = Result<T, Box<CallError>>> + Send + 'static,
        T: Send
    {
        let nvim = self.clone();

        self.spawn(async move {
            let res = nvim.timeout(f).await;
            if let Err(ref err) = res {
                if let Ok(e) = NormalError::try_from(err) {
                    e.print(&nvim).await;
                } else {
                    res.report_err();
                }
            }
        })
    }

    /// Shutdown this neovim session by executing the relevant autocommands, and then closing our
    /// RPC channel with the Neovim instance.
    pub async fn shutdown(&self) {
        self.timeout(self.command("doau VimLeavePre|doau VimLeave")).await.report_err();

        let chan = self
            .timeout(self.get_api_info())
            .await
            .ok_and_report()
            .and_then(|v| v[0].as_i64())
            .expect("Couldn't retrieve current channel for closing");

        let res = self.timeout(self.command(&format!("cal chanclose({})", chan))).await;
        if let Err(ref e) = res {
            if let SessionError::CallError(ref e) = *e {
                if let CallError::DecodeError(ref e, _) = **e {
                    if let DecodeError::ReaderError(_) = **e {
                        // It's expected that we'll fail to read the response to this
                        return;
                    }
                }
            }
            res.report_err();
        }
    }

    /// A helper for checking if nvim is currently blocked waiting on user input or not
    pub fn is_blocked(&self) -> bool {
        let &(_, ref blocked) = &self.block_on(self.get_mode()).unwrap()[1];

        blocked.as_bool().unwrap()
    }
}

impl Deref for NvimSession {
    type Target = Neovim;

    fn deref(&self) -> &Self::Target {
        &self.nvim
    }
}

impl DerefMut for NvimSession {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.nvim
    }
}

/// Wrap a future with a timeout, and spawn it on this session's tokio runtime, then report any
/// resulting errors to the console.
#[macro_export] macro_rules! spawn_timeout {
    ($nvim:ident.$fn:ident($( $a:expr ),*)) => {
        let nvim = $nvim.clone();
        $nvim.spawn_timeout(async move { nvim.$fn($( $a ),*).await })
    };
}

/// Wrap a future with a timeout, and spawn it on this session's tokio runtime, then report any
/// non-normal (see `NormalError` for more info) errors to the console. Any normal errors will be
/// printed as error messages in Neovim.
#[macro_export] macro_rules! spawn_timeout_user_err {
    ($nvim:ident.$fn:ident($( $a:expr ),*)) => {
        let nvim = $nvim.clone();
        $nvim.spawn_timeout_user_err(async move { nvim.$fn($( $a ),*).await })
    }
}

pub fn start<'a>(
    handler: NvimHandler,
    nvim_bin_path: Option<String>,
    timeout: Option<Duration>,
    args_for_neovim: Vec<String>,
) -> result::Result<(NvimSession, BoxFuture<'a, Result<(), Box<LoopError>>>), NvimInitError> {
    let mut cmd = if let Some(path) = nvim_bin_path {
        Command::new(path)
    } else {
        Command::new("nvim")
    };

    cmd.arg("--embed")
        .arg("--cmd")
        .arg("set termguicolors")
        .arg("--cmd")
        .arg("let g:GtkGuiLoaded = 1")
        .arg("--cmd")
        .arg(&format!("let &rtp.=',{}'",
                      env::var("NVIM_GTK_RUNTIME_PATH").unwrap_or(env!("RUNTIME_PATH").into())))
        .stderr(Stdio::inherit())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());

    #[cfg(target_os = "windows")]
    set_windows_creation_flags(&mut cmd);

    if let Some(nvim_config) = NvimConfig::config_path() {
        if let Some(path) = nvim_config.to_str() {
            cmd.arg("--cmd").arg(format!("source {}", path));
        }
    }

    for arg in args_for_neovim {
        cmd.arg(arg);
    }

    NvimSession::new_child(cmd, handler, timeout.unwrap_or(Duration::from_secs(10)))
}

pub async fn post_start_init(
    nvim: NvimSession,
    cols: NonZeroI64,
    rows: NonZeroI64,
    input_data: Option<String>,
) -> Result<(), NvimInitError> {
    let mut version_info: Vec<(Value, Value)> = vec![
        ("major".into(), env!("CARGO_PKG_VERSION_MAJOR").into()),
        ("minor".into(), env!("CARGO_PKG_VERSION_MINOR").into()),
        ("patch".into(), env!("CARGO_PKG_VERSION_PATCH").into()),
    ];
    if let Some(git_commit) = option_env!("GIT_COMMIT") {
        version_info.push(("commit".into(), git_commit.into()));
    }

    nvim.timeout(nvim.set_client_info(
        env!("CARGO_PKG_NAME"),
        version_info,
        "ui",
        vec![],
        vec![
            ("license".into(), env!("CARGO_PKG_LICENSE").into()),
        ]))
        .await.map_err(NvimInitError::new_post_init)?;

    nvim.timeout(nvim.ui_attach(
            cols.get(),
            rows.get(),
            UiAttachOptions::new()
            .set_popupmenu_external(true)
            .set_tabline_external(true)
            .set_linegrid_external(true)
            .set_hlstate_external(true)
            .set_termcolors_external(true)
        ))
        .await
        .map_err(NvimInitError::new_post_init)?;

    nvim.timeout(nvim.command("runtime! ginit.vim")).await.map_err(NvimInitError::new_post_init)?;

    if let Some(input_data) = input_data {
        let buf = nvim.timeout(nvim.get_current_buf()).await.ok_and_report();

        if let Some(buf) = buf {
            nvim.timeout(buf.set_lines(
                0,
                0,
                true,
                input_data.lines().map(|l| l.to_owned()).collect()
            )).await.report_err();
        }
    }

    Ok(())
}

use std::convert::{TryFrom, TryInto};
use std::error::Error;

use log::error;

use nvim_rs::error::CallError;

use crate::nvim::{NvimSession, SessionError};

pub trait CallErrorExt {
    fn print(&self);
}
impl CallErrorExt for CallError {
    fn print(&self) {
        error!("Error in last Neovim request: {self}");
        error!("Caused by: {:?}", self.source());
    }
}

pub trait ErrorReport<T> {
    fn report_err(&self);

    fn ok_and_report(self) -> Option<T>;
}

impl<T> ErrorReport<T> for Result<T, Box<CallError>> {
    fn report_err(&self) {
        if let Err(err) = self {
            err.print();
        }
    }

    fn ok_and_report(self) -> Option<T> {
        self.report_err();
        Some(self.unwrap())
    }
}

impl<T> ErrorReport<T> for Result<T, SessionError> {
    fn report_err(&self) {
        if let Err(err) = self {
            match *err {
                SessionError::CallError(ref e) => e.print(),
                SessionError::TimeoutError(ref e) => {
                    panic!("Neovim request {:?} timed out", e.source());
                }
            }
        }
    }

    fn ok_and_report(self) -> Option<T> {
        self.report_err();
        self.ok()
    }
}

/// An error from an neovim request that is part of neovim's regular operation, potentially with an
/// error message that's intended to potentially be displayed to the user as an error in the
/// messages buffer
#[derive(PartialEq, Eq, Debug)]
pub enum NormalError<'a> {
    /// A neovim request was interrupted by the user
    KeyboardInterrupt,
    /// A neovim error with a message for the messages buffer
    Message {
        source: &'a str,
        message: &'a str,
        code: u32,
    },
}

impl NormalError<'_> {
    /// Print an error message to neovim's message buffer, if we have one
    pub async fn print(&self, nvim: &NvimSession) {
        if let Self::Message { message, .. } = self {
            // TODO: Figure out timeout situation, in the mean time just disable timeouts here
            if let Err(e) = nvim.err_writeln(message).await {
                error!("Failed to print error message \"{self:?}\" in nvim: {e}");
            }
        }
    }

    /// Check if this error has the given code
    pub fn has_code(&self, code: u32) -> bool {
        match self {
            Self::Message { code: our_code, .. } => *our_code == code,
            _ => false,
        }
    }
}

impl<'a> TryFrom<&'a CallError> for NormalError<'a> {
    type Error = ();

    fn try_from(err: &'a CallError) -> Result<Self, Self::Error> {
        if let CallError::NeovimError(code, message) = err {
            if *code != Some(0) {
                return Err(());
            }

            if message == "Keyboard interrupt" {
                return Ok(Self::KeyboardInterrupt);
            } else if let Some(message) = message.strip_prefix("Vim(") {
                let (source, message) = match message.split_once("):") {
                    Some((source, message)) => (source, message),
                    None => return Err(()),
                };

                let code = match message
                    .strip_prefix('E')
                    .and_then(|message| message.split_once(':'))
                    .map(|(message, _)| message)
                    .and_then(|message| message.parse::<u32>().ok())
                {
                    Some(code) => code,
                    None => return Err(()),
                };

                return Ok(Self::Message {
                    source,
                    message,
                    code,
                });
            }
        }

        Err(())
    }
}

impl<'a> TryFrom<&'a SessionError> for NormalError<'a> {
    type Error = ();

    fn try_from(err: &'a SessionError) -> Result<Self, Self::Error> {
        if let SessionError::CallError(err) = err {
            err.as_ref().try_into()
        } else {
            Err(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normal_error() {
        macro_rules! test {
            ( $( $in_str:literal == $expr:expr );*; ) => {
                let mut error;
                $(
                    error = CallError::NeovimError(Some(0), $in_str.into());
                    assert_eq!(NormalError::try_from(&error), $expr)
                );*
            }
        }

        test! {
            "Vim(source):E325: ATTENTION" ==
                Ok(NormalError::Message {
                    source: "source",
                    message: "E325: ATTENTION",
                    code: 325,
                });
            "source): E325: ATTENTION" == Err(());      // Missing Vim(
            "Vim(source:E325: ATTENTION" == Err(());    // 1st : should be ):
            "Vim(source):EXXX: ATTENTION" == Err(());   // Invalid error code
            "Vim(source):325: ATTENTION" == Err(());    // Missing E prefix
            "Vim(source)E325: ATTENTION" == Err(());    // Missing 1st :
            "Vim(source):E325 ATTENTION" == Err(());    // Missing 2nd :
            "Keyboard interrupt" == Ok(NormalError::KeyboardInterrupt);
        }
    }
}

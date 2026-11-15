use crate::errors::*;
use crate::Ignore;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MidiInputPort {
    pub(crate) id: u32,
    pub(crate) name: String,
}

impl MidiInputPort {
    pub fn id(&self) -> String {
        self.name.clone()
    }
}

pub struct MidiInput {
    ignore_flags: Ignore,
}

impl MidiInput {
    pub fn new(_client_name: &str) -> Result<Self, InitError> {
        Ok(MidiInput {
            ignore_flags: Ignore::None,
        })
    }

    pub(crate) fn ports_internal(&self) -> Vec<crate::common::MidiInputPort> {
        Vec::new()
    }

    pub fn ignore(&mut self, flags: Ignore) {
        self.ignore_flags = flags;
    }

    pub fn port_count(&self) -> usize {
        0
    }

    pub fn port_name(&self, port: &MidiInputPort) -> Result<String, PortInfoError> {
        Ok(port.name.clone())
    }

    pub fn connect<F, T: Send + 'static>(
        self,
        _port: &MidiInputPort,
        _port_name: &str,
        _callback: F,
        _data: T,
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
    where
        F: FnMut(u64, &[u8], &mut T) + Send + 'static,
    {
        Err(ConnectError::new(ConnectErrorKind::Other("No MIDI devices available on Android dummy backend"), self))
    }

    pub fn create_virtual<F, T: Send + 'static>(
        self,
        _port_name: &str,
        _callback: F,
        _data: T,
    ) -> Result<MidiInputConnection<T>, ConnectError<Self>>
    where
        F: FnMut(u64, &[u8], &mut T) + Send + 'static,
    {
        Err(ConnectError::new(ConnectErrorKind::Other("Virtual MIDI ports not supported on Android"), self))
    }
}

pub struct MidiInputConnection<T> {
    ignore_flags: Ignore,
    user_data: Option<T>,
}

impl<T> MidiInputConnection<T> {
    pub fn close(mut self) -> (MidiInput, T) {
        let ignore_flags = self.ignore_flags;
        let data = self.user_data.take().unwrap();
        (MidiInput { ignore_flags }, data)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MidiOutputPort {
    pub(crate) id: u32,
    pub(crate) name: String,
}

impl MidiOutputPort {
    pub fn id(&self) -> String {
        self.name.clone()
    }
}

pub struct MidiOutput {}

impl MidiOutput {
    pub fn new(_client_name: &str) -> Result<Self, InitError> {
        Ok(MidiOutput {})
    }

    pub(crate) fn ports_internal(&self) -> Vec<crate::common::MidiOutputPort> {
        Vec::new()
    }

    pub fn port_count(&self) -> usize {
        0
    }

    pub fn port_name(&self, port: &MidiOutputPort) -> Result<String, PortInfoError> {
        Ok(port.name.clone())
    }

    pub fn connect(
        self,
        _port: &MidiOutputPort,
        _port_name: &str,
    ) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
        Err(ConnectError::new(ConnectErrorKind::Other("No MIDI devices available on Android dummy backend"), self))
    }

    pub fn create_virtual(
        self,
        _port_name: &str,
    ) -> Result<MidiOutputConnection, ConnectError<Self>> {
        Err(ConnectError::new(ConnectErrorKind::Other("Virtual MIDI ports not supported on Android"), self))
    }
}

pub struct MidiOutputConnection {}

impl MidiOutputConnection {
    pub fn close(self) -> MidiOutput {
        MidiOutput {}
    }

    pub fn send(&mut self, _message: &[u8]) -> Result<(), SendError> {
        Err(SendError::Other("No MIDI devices available on Android dummy backend"))
    }
}

//! The WASI 0.3 functions a guest kernel imports beyond the demo's: the wall
//! clock, randomness, an empty environment, no standard input, and no
//! terminals. None gives a guest kernel any authority.

use wcmp::{Access, FutureReader, Resource, StreamReader};

use crate::platform;
use crate::sapling::{KernelHost, Sapling};
use crate::wasi_bindings::wasi::cli::{
    environment, exit, stdin, terminal_input, terminal_output, terminal_stderr, terminal_stdin,
    terminal_stdout,
};
use crate::wasi_bindings::wasi::clocks::{system_clock, types as clock_types};
use crate::wasi_bindings::wasi::random::{insecure_seed, random};

impl clock_types::Host for KernelHost {}
impl crate::wasi_bindings::wasi::cli::types::Host for KernelHost {}

impl system_clock::Host for KernelHost {
    fn now(&mut self) -> system_clock::Instant {
        let millis = platform::wall_millis();
        let seconds = (millis / 1000.0).floor();
        system_clock::Instant {
            seconds: seconds as i64,
            nanoseconds: ((millis - seconds * 1000.0) * 1_000_000.0) as u32,
        }
    }

    fn get_resolution(&mut self) -> u64 {
        1_000_000
    }
}

impl random::Host for KernelHost {
    fn get_random_bytes(&mut self, max_len: u64) -> Vec<u8> {
        let mut bytes = vec![0; max_len.min(1 << 16) as usize];
        getrandom::getrandom(&mut bytes).expect("the platform has randomness");
        bytes
    }

    fn get_random_u64(&mut self) -> u64 {
        let mut bytes = [0; 8];
        getrandom::getrandom(&mut bytes).expect("the platform has randomness");
        u64::from_le_bytes(bytes)
    }
}

impl insecure_seed::Host for KernelHost {
    fn get_insecure_seed(&mut self) -> (u64, u64) {
        (
            random::Host::get_random_u64(self),
            random::Host::get_random_u64(self),
        )
    }
}

impl environment::Host for KernelHost {
    fn get_environment(&mut self) -> Vec<(String, String)> {
        Vec::new()
    }

    fn get_arguments(&mut self) -> Vec<String> {
        Vec::new()
    }

    fn get_initial_cwd(&mut self) -> Option<String> {
        None
    }
}

impl exit::Host for KernelHost {
    fn exit(&mut self, status: Result<(), ()>) {
        panic!("the guest kernel exited with {status:?}");
    }

    fn exit_with_code(&mut self, status_code: u8) {
        panic!("the guest kernel exited with code {status_code}");
    }
}

impl stdin::Host for KernelHost {}

impl<T: 'static> stdin::HostWithStore<T> for Sapling {
    fn read_via_stream(
        mut host: Access<'_, T, Self>,
    ) -> (StreamReader<u8>, FutureReader<Result<(), stdin::ErrorCode>>) {
        // Standard input is empty: a stream that ends at once, and a read
        // that succeeds.
        let stream =
            StreamReader::new(host.store(), Vec::<u8>::new()).expect("the store makes a stream");
        let done = FutureReader::new(host.store(), async { Ok::<_, wcmp::Error>(Ok(())) })
            .expect("the store makes a future");
        (stream, done)
    }
}

impl terminal_input::HostTerminalInput for KernelHost {
    fn drop(&mut self, _rep: Resource<terminal_input::TerminalInput>) -> wcmp::Result<()> {
        Ok(())
    }
}
impl terminal_input::Host for KernelHost {}

impl terminal_output::HostTerminalOutput for KernelHost {
    fn drop(&mut self, _rep: Resource<terminal_output::TerminalOutput>) -> wcmp::Result<()> {
        Ok(())
    }
}
impl terminal_output::Host for KernelHost {}

impl terminal_stdin::Host for KernelHost {
    fn get_terminal_stdin(&mut self) -> Option<Resource<terminal_input::TerminalInput>> {
        None
    }
}

impl terminal_stdout::Host for KernelHost {
    fn get_terminal_stdout(&mut self) -> Option<Resource<terminal_output::TerminalOutput>> {
        None
    }
}

impl terminal_stderr::Host for KernelHost {
    fn get_terminal_stderr(&mut self) -> Option<Resource<terminal_output::TerminalOutput>> {
        None
    }
}

extern crate xml;
extern crate serial;
extern crate time;

use std::path::Path;
use std::error::Error as ErrorT;

use xml::EventReader;

use serial::prelude::*;
use serial::posix::TTYPort;
use serial::PortSettings;
use serial::BaudRate::*;
use time::Duration;

type Error = Box<ErrorT>;

fn init_serial() -> Result<TTYPort, Error> {
    let settings = PortSettings {
        baud_rate: Baud57600,
        ..PortSettings::default()
    };
    let mut port = try!(TTYPort::open(Path::new("/dev/ttyUSB0")));
    try!(port.configure(&settings));
    try!(port.set_timeout(Duration::days(1)));
    Ok(port)
}

fn main_with_result() -> Result<(), Error> {
    let serial_input = try!(init_serial());

    let mut event_reader = EventReader::new(serial_input);

    for event in event_reader.events() {
        println!("{:?}", event);
    }

    Ok(())
}

fn main() {
    loop {
        if let Err(e) = main_with_result() {
            println!("{}", e);
        }
    }
}

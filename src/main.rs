extern crate xml;
extern crate serial;
extern crate time;
#[macro_use] extern crate log;
extern crate festivus_client;

use std::path::Path;
use std::io::Read;
use std::collections::HashMap;
use std::error::Error as ErrorT;

use xml::EventReader;
use xml::reader::events::XmlEvent;
use xml::reader::events::XmlEvent::*;
use xml::name::OwnedName;

use serial::prelude::*;
use serial::posix::TTYPort;
use serial::PortSettings;
use serial::BaudRate::*;
use time::Duration;

use festivus_client::Festivus;

use Parser::*;

type Error = Box<ErrorT>;

enum Parser {
    Top(&'static str, Vec<Parser>),
    Tag(&'static str, Vec<Parser>),
    Contents(&'static str, &'static str)
}

impl Parser {
    fn tag_name(&self) -> &'static str {
        match *self {
            Top(x, _) | Tag(x, _) | Contents(x, _) => x
        }
    }
}

trait EventReaderExt {
    /// Next important tag.
    fn next_tag(&mut self) -> XmlEvent;
    /// Consume tags until the given end tag is reached.
    fn read_to_tag_end(&mut self, tag: &str);
}

impl<T> EventReaderExt for EventReader<T> where T: Read {
    fn next_tag(&mut self) -> XmlEvent {
        match self.next() {
            // Ignored tag types.
            StartDocument { .. } |
            ProcessingInstruction { .. } |
            CData(..) |
            Comment(..) |
            Whitespace(..) => self.next_tag(),
            x => {
                info!("Read tag: {:?}", x);
                x
            }
        }
    }
    fn read_to_tag_end(&mut self, tag: &str) {
        loop {
            if let EndElement { ref name, .. } = self.next_tag() {
                if &name.local_name[..] == tag {
                    info!("Closed </{}>", tag);
                    break;
                }
            }
        }
    }
}

fn is_start_tag(ev: &XmlEvent, tag_name: &str) -> bool {
    match *ev {
        StartElement { ref name, .. } => name_matches_str(name, tag_name),
        _ => false
    }
}

fn name_matches_str(name: &OwnedName, str_name: &str) -> bool {
    &name.local_name == str_name
}

trait ToResult {
    fn to_result(self) -> Result<(), ()>;
}
impl ToResult for bool {
    fn to_result(self) -> Result<(), ()> {
        match self {
            true => Ok(()),
            false => Err(())
        }
    }
}

// Responsibilities: parsers are responsible for parsing the *inside and end* of their tag,
// having had their start parsed by their parent element. The exception to this is
// the `Top` tag which parses its own start.
fn run_parser<T>(src: &mut EventReader<T>, parser: &Parser)
    -> Result<HashMap<&'static str, String>, ()>
    where T: Read
{
    match *parser {
        Top(tag, ref subparsers) => {
            try!(is_start_tag(&src.next_tag(), tag).to_result());

            // Parse the inside and end of the tag.
            parse_tag(src, tag, subparsers)
        }

        Tag(tag, ref subparsers) => parse_tag(src, tag, subparsers),

        Contents(tag, key_name) => {
            let mut result = HashMap::new();
            match src.next_tag() {
                Characters(tag_content) => { result.insert(key_name, tag_content); },
                _ => return Err(())
            }
            src.read_to_tag_end(tag);
            Ok(result)
        }
    }
}

fn parse_tag<T>(src: &mut EventReader<T>, tag: &'static str, subparsers: &[Parser])
    -> Result<HashMap<&'static str, String>, ()>
    where T: Read
{
    let mut result = HashMap::new();
    for subparser in subparsers {
        debug!("Looking for a match for <{}>", subparser.tag_name());
        // Loop through tokens until a match for this subparser is found.
        loop {
            match src.next_tag() {
                StartElement { name: ref tag_name, .. } => {
                    // Tag matches sub-parser.
                    if name_matches_str(tag_name, subparser.tag_name()) {
                        debug!("Matched <{}>", subparser.tag_name());
                        match run_parser(src, subparser) {
                            Ok(subresult) => { result.extend(subresult); },
                            Err(()) => return Err(())
                        }
                        break;
                    }
                    // Otherwise, skip the tag.
                    else {
                        src.read_to_tag_end(&tag_name.local_name)
                    }
                }
                _ => return Err(())
            }
        }
    }
    // Read to end of tag.
    src.read_to_tag_end(tag);
    Ok(result)
}

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

    let parser =
        Top("msg", vec![
            Contents("time", "time"),
            Contents("tmpr", "temperature"),
            Tag("ch1", vec![Contents("watts", "total")]),
            Tag("ch2", vec![Contents("watts", "hotwater")]),
            Tag("ch3", vec![Contents("watts", "solar")])
        ]);

    let client = Festivus::new("http://localhost:3000");

    loop {
        let data = match run_parser(&mut event_reader, &parser) {
            Ok(x) => x,
            Err(_) => {
                println!("(historical message - ignored)");
                continue;
            }
        };
        println!("{:?}", data);

        let total = try!(data["total"].parse());
        let hot_water = try!(data["hot_water"].parse());
        let solar = try!(data["solar"].parse());

        if let Err(e) = client.insert(total, hot_water, solar) {
            println!("Error connecting to Festivus: {:?}", e);
        }
    }
}

fn main() {
    loop {
        if let Err(e) = main_with_result() {
            println!("{}", e);
        }
    }
}

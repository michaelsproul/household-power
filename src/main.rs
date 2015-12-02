extern crate xml;
extern crate serial;
#[macro_use] extern crate log;
extern crate festivus_client;

use std::path::Path;
use std::io::Read;
use std::collections::HashMap;
use std::error::Error;

use xml::EventReader;
use xml::reader::XmlEvent;
use xml::reader::XmlEvent::*;
use xml::reader::Error as XmlError;
use xml::name::OwnedName;

use serial::prelude::*;
use serial::posix::TTYPort;
use serial::PortSettings;
use serial::BaudRate::*;
use std::time::Duration;

use festivus_client::Festivus;

use Parser::*;

const ONE_DAY: u64 = 60 * 60 * 24;

/// Convert a String to a Box<Error>.
fn string_error<T>(s: String) -> Result<T, Box<Error>> {
    let err: Box<Error + Send + Sync> = From::from(s);
    Err(err as Box<Error>)
}

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
    fn next_tag(&mut self) -> Result<XmlEvent, XmlError>;
    /// Consume tags until the given end tag is reached.
    fn read_to_tag_end(&mut self, tag: &str);
}

impl<T> EventReaderExt for EventReader<T> where T: Read {
    fn next_tag(&mut self) -> Result<XmlEvent, XmlError> {
        self.next().and_then(|tag| {
            match tag {
                // Ignored tag types.
                StartDocument { .. } |
                ProcessingInstruction { .. } |
                CData(..) |
                Comment(..) |
                Whitespace(..) => self.next_tag(),
                // Anything else (not ignored).
                x => {
                    info!("Read tag: {:?}", x);
                    Ok(x)
                }
            }
        })
    }

    fn read_to_tag_end(&mut self, tag: &str) {
        loop {
            // FIXME: infinite loop on error?
            if let Ok(EndElement { ref name, .. }) = self.next_tag() {
                if &name.local_name[..] == tag {
                    info!("Closed </{}>", tag);
                    break;
                }
            }
        }
    }
}

fn name_matches_str(name: &OwnedName, str_name: &str) -> bool {
    &name.local_name == str_name
}

// Parsers are responsible for parsing the *inside and end* of their tag,
// having had their start parsed by their parent element. The exception to this is
// the `Top` tag which parses its own start.
fn run_parser<T: Read>(src: &mut EventReader<T>, parser: &Parser)
    -> Result<HashMap<&'static str, String>, Box<Error>>
{
    match *parser {
        Top(tag, ref subparsers) => {
            // Grab the start tag.
            let start_tag = try!(src.next_tag());
            match start_tag {
                // If we have the correct start tag, all is well.
                StartElement { ref name, .. } if name_matches_str(name, tag) => (),
                // If we have another start tag, read to the end of it and bail.
                StartElement { ref name, .. } => {
                    src.read_to_tag_end(&name.local_name);
                    return string_error(format!("Wrong start tag: {:?}", name));
                }
                // Anything else is bad.
                e => return string_error(format!("Junk event: {:?}", e))
            }

            // Parse the inside and end of the tag.
            parse_tag(src, tag, subparsers)
        }

        Tag(tag, ref subparsers) => parse_tag(src, tag, subparsers),

        Contents(tag, key_name) => {
            let mut result = HashMap::new();
            match src.next_tag() {
                Ok(Characters(tag_content)) => { result.insert(key_name, tag_content); },
                _ => return string_error(format!("Tag contents not found for tag parser"))
            }
            src.read_to_tag_end(tag);
            Ok(result)
        }
    }
}

fn parse_tag<T: Read>(src: &mut EventReader<T>, tag: &'static str, subparsers: &[Parser])
    -> Result<HashMap<&'static str, String>, Box<Error>>
{
    let mut result = HashMap::new();
    for subparser in subparsers {
        debug!("Looking for a match for <{}>", subparser.tag_name());
        // Loop through tokens until a match for this subparser is found.
        loop {
            match src.next_tag() {
                Ok(StartElement { name: ref tag_name, .. }) => {
                    // Tag matches sub-parser.
                    if name_matches_str(tag_name, subparser.tag_name()) {
                        debug!("Matched <{}>", subparser.tag_name());
                        let subresult = try!(run_parser(src, subparser));
                        result.extend(subresult);
                        break;
                    }
                    // Otherwise, skip the tag.
                    else {
                        src.read_to_tag_end(&tag_name.local_name)
                    }
                }
                _ => return string_error(format!("XML stream out of sync with parser"))
            }
        }
    }
    // Read to end of tag.
    src.read_to_tag_end(tag);
    Ok(result)
}

fn init_serial() -> Result<TTYPort, Box<Error>> {
    let settings = PortSettings {
        baud_rate: Baud57600,
        ..PortSettings::default()
    };
    let mut port = try!(TTYPort::open(Path::new("/dev/ttyUSB0")));
    try!(port.configure(&settings));
    try!(port.set_timeout(Duration::new(ONE_DAY, 0)));
    Ok(port)
}

fn main_with_result() -> Result<(), Box<Error>> {
    let serial_input = try!(init_serial());

    let mut event_reader = EventReader::new(serial_input);

    let parser =
        Top("msg", vec![
            Contents("time", "time"),
            Contents("tmpr", "temperature"),
            Tag("ch1", vec![Contents("watts", "total")]),
            Tag("ch2", vec![Contents("watts", "hot_water")]),
            Tag("ch3", vec![Contents("watts", "solar")])
        ]);

    let client = Festivus::new("http://localhost:3000");

    loop {
        let data = match run_parser(&mut event_reader, &parser) {
            Ok(x) => x,
            Err(e) => {
                println!("Parse error: {}", e);
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

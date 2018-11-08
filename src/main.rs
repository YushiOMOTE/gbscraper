extern crate curl;
extern crate pest;
extern crate scraper;
#[macro_use]
extern crate pest_derive;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate regex;
#[macro_use]
extern crate lazy_static;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_yaml;
extern crate structopt;

use pest::Parser;
#[derive(Parser)]
#[grammar = "inst.pest"]
struct InstParser;

use curl::easy::Easy;
use regex::Regex;
use scraper::{Html, Selector};
use scraper::element_ref::ElementRef;
use structopt::StructOpt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::fs::File;
use std::io::prelude::*;

#[derive(StructOpt)]
struct Opt {
    #[structopt(short = "u", long = "url")]
    url: Option<String>,

    #[structopt(name = "OUTPUT", parse(from_os_str))]
    output: PathBuf,
}

#[derive(Debug)]
struct Error(String);

type Result<T> = std::result::Result<T, Error>;

impl From<String> for Error {
    fn from(s: String) -> Error {
        Error(s)
    }
}

impl From<std::num::ParseIntError> for Error {
    fn from(s: std::num::ParseIntError) -> Error {
        Error(s.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(s: std::io::Error) -> Error {
        Error(s.to_string())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum Time {
    One(usize),
    Two(usize, usize),
}

#[derive(Debug, Serialize, Deserialize)]
struct Instruction {
    code: u16,
    operator: String,
    operands: Vec<String>,
    bits: usize,
    size: usize,
    time: Time,
    z: String,
    n: String,
    h: String,
    c: String,
}

fn modify(s: &str) -> String {
    let s = s.to_lowercase();
    let re = Regex::new(r"(?P<v>[0-9a-zA-Z]+)h").expect("Invalid regex");

    re.replace_all(&s, "0x$v")
        .replace("d8", "n")
        .replace("d16", "nn")
        .replace("a8", "n")
        .replace("a16", "nn")
        .replace("r8", "n")
}

lazy_static! {
    static ref SUFFIX: HashMap<&'static str, usize> = {
        let mut suffix = HashMap::new();
        suffix.insert("#ff99cc", 0);
        suffix.insert("#ffcc99", 0);
        suffix.insert("#ccccff", 8);
        suffix.insert("#ccffcc", 16);
        suffix.insert("#ffff99", 8);
        suffix.insert("#ffcccc", 16);
        suffix.insert("#80ffff", 8);
        suffix
    };
}

fn parse_time(s: &str) -> Time {
    if s.contains("/") {
        let mut nums = s.split("/");
        Time::Two(
            nums.next()
                .expect("Incomplete time")
                .parse()
                .expect("Bad time"),
            nums.next()
                .expect("Incomplete time")
                .parse()
                .expect("Bad time"),
        )
    } else {
        Time::One(s.parse().expect("Bad time"))
    }
}

fn parse_table(table: ElementRef, op_prefix: u16) -> Vec<Instruction> {
    let mut vec = Vec::new();

    let mut x = 0;
    let mut y = 0;

    let sel = Selector::parse("td").expect("Select failed");

    for item in table.select(&sel) {
        let bits = *SUFFIX
            .get(item.value().attr("bgcolor").unwrap_or(""))
            .unwrap_or(&0);

        let s = item.inner_html();

        let code = ((y - 1) << 4 | (x - 1)) as u16 | (op_prefix << 8);

        x += 1;
        if x % 17 == 0 {
            y += 1;
            x = 0;
        }

        let mut p = match InstParser::parse(Rule::Instruction, &s) {
            Ok(p) => p,
            Err(e) => {
                debug!("Skipping: {}", e);
                continue;
            }
        };

        let mnem = p.next().expect("No mnemonic");

        let mut ops = mnem.into_inner();
        let operator = ops.next().expect("No operator").as_str().to_lowercase();
        let operands = ops.map(|p| modify(p.as_str())).collect::<Vec<_>>();

        let size: usize = p.next()
            .expect("No size")
            .as_str()
            .parse()
            .expect("Bad size");
        let time = parse_time(p.next().expect("No time").as_str());
        let flag = p.next().expect("No flag");

        let mut flag = flag.into_inner();
        let z = flag.next().expect("No z flag").as_str().into();
        let n = flag.next().expect("No n flag").as_str().into();
        let h = flag.next().expect("No h flag").as_str().into();
        let c = flag.next().expect("No c flag").as_str().into();

        info!(
            "{:02x}: {} {:?}: bits: {}, size: {}, time: {:?}, flags: z[{}],n[{}],h[{}],c[{}]",
            code, operator, operands, bits, size, time, z, n, h, c
        );

        vec.push(Instruction {
            code,
            operator,
            operands,
            bits,
            size,
            time,
            z,
            n,
            h,
            c,
        })
    }

    vec
}

fn main() -> Result<()> {
    let opt = Opt::from_args();

    env_logger::init();

    let mut buf = Vec::new();
    let mut handle = Easy::new();

    handle
        .url(&opt.url
            .unwrap_or("http://www.pastraiser.com/cpu/gameboy/gameboy_opcodes.html".into()))
        .map_err(|e| e.to_string())?;
    {
        let mut transfer = handle.transfer();
        transfer
            .write_function(|d| {
                buf.extend_from_slice(d);
                Ok(d.len())
            })
            .map_err(|e| e.to_string())?;
        transfer.perform().map_err(|e| e.to_string())?;
    }

    let doc = String::from_utf8(buf).map_err(|e| e.to_string())?;
    let doc = Html::parse_document(&doc);

    let sel = Selector::parse("table").map_err(|_| Error("Select failed".into()))?;
    let mut it = doc.select(&sel);

    let mut insts = Vec::new();

    if let Some(table) = it.next() {
        insts.extend(parse_table(table, 0));
    }
    if let Some(table) = it.next() {
        insts.extend(parse_table(table, 0xcb));
    }

    let insts = serde_yaml::to_string(&insts).expect("Pack error");

    let mut file = File::create(opt.output)?;
    file.write_all(insts.as_bytes())?;

    Ok(())
}

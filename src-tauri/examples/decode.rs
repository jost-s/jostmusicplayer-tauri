// Throwaway diagnostic: reproduce player.rs's decode path for a given file.
use std::fs::File;
use std::io::BufReader;
use std::panic::{self, AssertUnwindSafe};

use rodio::{Decoder, Source};

fn main() {
    let path = std::env::args().nth(1).expect("usage: decode <path>");
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        let file = File::open(&path).expect("File::open failed");
        let dec = Decoder::new(BufReader::new(file)).map_err(|e| format!("decode: {e}"))?;
        let total = dec.total_duration();
        let count = dec.count();
        Ok::<_, String>((total, count))
    }));
    match result {
        Ok(Ok((total, count))) => println!("OK total_duration={total:?} samples={count}"),
        Ok(Err(e)) => println!("ERR {e}"),
        Err(_) => println!("PANIC during decode"),
    }
}

//! Prints an antenna's catalogue entry, or a summary of the catalogue:
//! `cargo run --example catalogue -- LEIAS10`.

use gnss_antennas::Catalogue;

fn main() {
    let c = Catalogue::igs();
    let codes: Vec<String> = std::env::args().skip(1).collect();
    if codes.is_empty() {
        let all: Vec<_> = c.antennas().collect();
        let drawn = all.iter().filter(|a| a.drawing.is_some()).count();
        let no_arp_mark = all
            .iter()
            .filter(|a| a.drawing.as_deref().is_some_and(|d| !d.contains("ARP")))
            .map(|a| a.code.as_str())
            .collect::<Vec<_>>();
        println!(
            "{} antennas, {} with an ARP, {} with a drawing; drawings without an ARP mark: {:?}",
            all.len(),
            all.iter().filter(|a| a.arp.is_some()).count(),
            drawn,
            no_arp_mark
        );
        return;
    }
    for code in codes {
        match c.antenna(&code) {
            None => println!("{code}: not an IGS antenna code"),
            Some(a) => {
                let feature = |f: &Option<gnss_antennas::Feature>| {
                    f.as_ref().map_or("not given".to_owned(), |f| {
                        format!("{} ({})", f.code, f.meaning.as_deref().unwrap_or("?"))
                    })
                };
                println!("{}: {}", a.code, a.description);
                println!("ARP: {}; north: {}", feature(&a.arp), feature(&a.north));
                println!("{}", a.drawing.as_deref().unwrap_or("(no drawing)"));
            }
        }
    }
}

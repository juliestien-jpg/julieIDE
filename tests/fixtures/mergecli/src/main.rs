use std::fs;
fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let mut names: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_dir()).collect();
    names.sort();
    let cancel = julie_core::CancelToken::new();
    for p in names {
        let rd = |n: &str| fs::read(p.join(n)).unwrap();
        let (b, o, t) = (rd("base"), rd("ours"), rd("theirs"));
        let (Ok(b), Ok(o), Ok(t)) = (String::from_utf8(b), String::from_utf8(o), String::from_utf8(t)) else { println!("{}\tSKIP", p.file_name().unwrap().to_string_lossy()); continue };
        let r = julie_diff::merge(&b, &o, &t, &cancel).unwrap();
        fs::write(p.join("out"), r.text.as_bytes()).unwrap();
        println!("{}\t{}", p.file_name().unwrap().to_string_lossy(), r.conflicts);
    }
}

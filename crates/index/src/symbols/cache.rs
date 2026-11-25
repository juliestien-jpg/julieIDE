//! Cache disque de l'index des symboles (`.juliestorm/index/symbols.bin`).
//!
//! Format binaire maison, petit-boutiste : en-tête (`JSYMIDX\0`, version),
//! fichiers (chemin relatif, date, taille, symboles), puis une empreinte FNV de
//! tout ce qui précède. Fichier illisible, tronqué, d'une autre version ou
//! d'empreinte fausse : ignoré sans message, l'index est reconstruit.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::model::{
    fnv, BaseRel, Decl, FileSymbols, ImportName, JsImport, JsModule, JsReexport, Lang, Span,
};
use super::store::{Stamp, SymbolIndex};
use super::SymbolKind;
use crate::error::Result;

pub(crate) const CACHE_FILE_NAME: &str = "symbols.bin";
const MAGIC: &[u8; 8] = b"JSYMIDX\0";
/// À augmenter à chaque changement du format **ou de l'extraction** (un cache
/// d'une extraction plus ancienne donnerait d'autres résultats).
const VERSION: u32 = 1;

pub(crate) fn cache_file(dir: &Path) -> PathBuf {
    dir.join(CACHE_FILE_NAME)
}

pub(crate) type CacheMap = HashMap<Box<str>, (Stamp, FileSymbols)>;

/// Relit le cache. **Accès disque.** `Err` ou rien de valide : on reconstruit.
pub(crate) fn load(dir: &Path) -> std::result::Result<CacheMap, ()> {
    let bytes = std::fs::read(cache_file(dir)).map_err(|_| ())?;
    decode(&bytes).ok_or(())
}

/// Écrit le cache (dossier créé au besoin, écriture atomique). **Accès disque.**
pub(crate) fn save(index: &SymbolIndex, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| julie_core::Error::io(dir, e))?;
    let bytes = encode(index);
    julie_core::file::write_atomic(cache_file(dir), &bytes)?;
    Ok(())
}

pub(crate) fn encode(index: &SymbolIndex) -> Vec<u8> {
    let mut w = Writer(Vec::with_capacity(1 << 20));
    w.0.extend_from_slice(MAGIC);
    w.u32(VERSION);
    let slots: Vec<_> = index
        .slots()
        .filter(|(_, s)| !s.from_memory && s.stamp != Stamp::default())
        .collect();
    w.u32(slots.len() as u32);
    for (_, slot) in slots {
        w.str(&slot.relative);
        w.u64(slot.stamp.mtime);
        w.u64(slot.stamp.size);
        write_symbols(&mut w, &slot.symbols);
    }
    let sum = fnv(&w.0);
    w.u64(sum);
    w.0
}

pub(crate) fn decode(bytes: &[u8]) -> Option<CacheMap> {
    if bytes.len() < MAGIC.len() + 4 + 4 + 8 {
        return None;
    }
    let (body, tail) = bytes.split_at(bytes.len() - 8);
    if fnv(body).to_le_bytes() != tail {
        return None;
    }
    let mut r = Reader { b: body, at: 0 };
    if r.take(8)? != MAGIC || r.u32()? != VERSION {
        return None;
    }
    let count = r.u32()? as usize;
    let mut map = HashMap::with_capacity(count.min(1 << 20));
    for _ in 0..count {
        let relative: Box<str> = r.str()?.into();
        let stamp = Stamp {
            mtime: r.u64()?,
            size: r.u64()?,
        };
        let symbols = read_symbols(&mut r)?;
        map.insert(relative, (stamp, symbols));
    }
    (r.at == body.len()).then_some(map)
}

fn write_symbols(w: &mut Writer, s: &FileSymbols) {
    w.u8(s.lang.code());
    w.u64(s.content_hash);
    w.u32(s.decls.len() as u32);
    for d in &s.decls {
        w.u8(d.kind.code());
        w.u8(d.flags);
        w.str(&d.name);
        w.str(&d.container);
        for v in [
            d.span.start,
            d.span.end,
            d.span.line,
            d.span.col,
            d.span.end_col,
        ] {
            w.u32(v);
        }
        w.u32(d.types.len() as u32);
        for t in d.types.iter() {
            w.str(t);
        }
        w.u32(d.bases.len() as u32);
        for (rel, t) in d.bases.iter() {
            w.u8(rel.code());
            w.str(t);
        }
    }
    w.u32(s.ref_keys.len() as u32);
    for k in s.ref_keys.iter() {
        w.u32(*k);
    }
    match &s.js {
        None => w.u8(0),
        Some(js) => {
            w.u8(1);
            w.u8(js.is_module as u8);
            w.u32(js.imports.len() as u32);
            for i in &js.imports {
                w.str(&i.local);
                w.str(&i.source);
                match &i.imported {
                    ImportName::Named(n) => {
                        w.u8(0);
                        w.str(n);
                    }
                    ImportName::Default => w.u8(1),
                    ImportName::Namespace => w.u8(2),
                }
            }
            w.u32(js.exports.len() as u32);
            for (a, b) in &js.exports {
                w.str(a);
                w.str(b);
            }
            w.u32(js.reexports.len() as u32);
            for re in &js.reexports {
                w.str(&re.source);
                match &re.names {
                    None => w.u8(0),
                    Some(names) => {
                        w.u8(1);
                        w.u32(names.len() as u32);
                        for (a, b) in names {
                            w.str(a);
                            w.str(b);
                        }
                    }
                }
            }
        }
    }
}

fn read_symbols(r: &mut Reader) -> Option<FileSymbols> {
    let lang = Lang::from_code(r.u8()?)?;
    let content_hash = r.u64()?;
    let n = r.u32()? as usize;
    let mut decls = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        let kind = SymbolKind::from_code(r.u8()?)?;
        let flags = r.u8()?;
        let name = r.str()?.into();
        let container = r.str()?.into();
        let span = Span {
            start: r.u32()?,
            end: r.u32()?,
            line: r.u32()?,
            col: r.u32()?,
            end_col: r.u32()?,
        };
        let nt = r.u32()? as usize;
        let mut types = Vec::with_capacity(nt.min(64));
        for _ in 0..nt {
            types.push(r.str()?.into());
        }
        let nb = r.u32()? as usize;
        let mut bases = Vec::with_capacity(nb.min(64));
        for _ in 0..nb {
            let rel = BaseRel::from_code(r.u8()?)?;
            bases.push((rel, r.str()?.into()));
        }
        decls.push(Decl {
            kind,
            flags,
            name,
            container,
            span,
            types: types.into_boxed_slice(),
            bases: bases.into_boxed_slice(),
        });
    }
    let nk = r.u32()? as usize;
    let mut keys = Vec::with_capacity(nk.min(1 << 16));
    for _ in 0..nk {
        keys.push(r.u32()?);
    }
    let js = match r.u8()? {
        0 => None,
        1 => {
            let is_module = r.u8()? != 0;
            let ni = r.u32()? as usize;
            let mut imports = Vec::with_capacity(ni.min(1024));
            for _ in 0..ni {
                let local = r.str()?.into();
                let source = r.str()?.into();
                let imported = match r.u8()? {
                    0 => ImportName::Named(r.str()?.into()),
                    1 => ImportName::Default,
                    2 => ImportName::Namespace,
                    _ => return None,
                };
                imports.push(JsImport {
                    local,
                    source,
                    imported,
                });
            }
            let ne = r.u32()? as usize;
            let mut exports = Vec::with_capacity(ne.min(1024));
            for _ in 0..ne {
                exports.push((r.str()?.into(), r.str()?.into()));
            }
            let nr = r.u32()? as usize;
            let mut reexports = Vec::with_capacity(nr.min(1024));
            for _ in 0..nr {
                let source = r.str()?.into();
                let names = match r.u8()? {
                    0 => None,
                    1 => {
                        let n = r.u32()? as usize;
                        let mut v = Vec::with_capacity(n.min(1024));
                        for _ in 0..n {
                            v.push((r.str()?.into(), r.str()?.into()));
                        }
                        Some(v)
                    }
                    _ => return None,
                };
                reexports.push(JsReexport { source, names });
            }
            Some(Box::new(JsModule {
                is_module,
                imports,
                exports,
                reexports,
            }))
        }
        _ => return None,
    };
    Some(FileSymbols {
        lang,
        content_hash,
        decls,
        ref_keys: keys.into_boxed_slice(),
        js,
    })
}

struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.b.get(self.at..end)?;
        self.at = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn str(&mut self) -> Option<&'a str> {
        let n = self.u32()? as usize;
        std::str::from_utf8(self.take(n)?).ok()
    }
}

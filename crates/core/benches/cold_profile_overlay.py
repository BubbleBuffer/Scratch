#!/usr/bin/env python3
"""Add the same feature-only timers to an archived master checkout.

Never copies candidate storage behavior into master. All replacements are
checked against the original master source; an unexpected source shape fails.
"""
import argparse
from pathlib import Path
import re
import shutil


def replace(text, old, new, count=1):
    actual = text.count(old)
    if actual != count:
        raise ValueError(f"expected {count} occurrences, found {actual}: {old[:90]!r}")
    return text.replace(old, new)


def guard(label):
    return f'#[cfg(feature = "storage-metrics")]\n        let _metric = crate::storage_metrics::span("{label}");\n'


def function(text, name, label):
    pattern = re.compile(r'(\bfn ' + re.escape(name) + r'(?:<[^{}]*?>)?\([^{}]*?\)(?:\s*->[^{}]*?)?\s*\{\n)')
    matches = list(pattern.finditer(text))
    if len(matches) != 1:
        raise ValueError(f"{name}: expected one function, found {len(matches)}")
    return pattern.sub(lambda match: match[1] + '        ' + guard(label), text, count=1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--candidate', type=Path, required=True)
    parser.add_argument('--master', type=Path, required=True)
    args = parser.parse_args()
    candidate, master = args.candidate.resolve(), args.master.resolve()
    if candidate == master or not (master / 'Cargo.toml').is_file():
        raise ValueError('need a distinct archived master checkout')
    if (master / '.git').exists():
        raise ValueError('overlay only an owned archive, never a git worktree')
    def edit(relative, transform):
        path = master / relative
        if path.is_symlink() or not path.resolve().is_relative_to(master):
            raise ValueError('unsafe overlay path')
        text = path.read_text()
        path.write_text(transform(text))
    feature = '\n[features]\nstorage-metrics = []\n'
    edit('crates/core/Cargo.toml', lambda text: text + feature if '[features]' not in text else (_ for _ in ()).throw(ValueError('master already has features')))
    edit('crates/core/src/lib.rs', lambda text: text + '\n#[cfg(feature = "storage-metrics")]\npub mod storage_metrics;\n')
    shutil.copyfile(candidate / 'crates/core/src/storage_metrics.rs', master / 'crates/core/src/storage_metrics.rs')
    def doc(text):
        text = function(text, 'from_bytes', 'pagedoc.from_bytes')
        text = replace(text, '        doc.import(snapshot)?;', '''        {
            #[cfg(feature = "storage-metrics")]
            let _metric = crate::storage_metrics::span("loro.import_initial_blob");
            #[cfg(feature = "storage-metrics")]
            crate::storage_metrics::add_bytes("loro.import_initial_blob", snapshot.len() as u64);
            doc.import(snapshot)?;
        }''')
        text = replace(text, '            doc.import_batch(updates)?;', '''            #[cfg(feature = "storage-metrics")]
            let _metric = crate::storage_metrics::span("loro.import_batch");
            #[cfg(feature = "storage-metrics")]
            crate::storage_metrics::add_bytes("loro.import_batch", updates.iter().map(|b| b.len() as u64).sum());
            doc.import_batch(updates)?;''')
        text = function(text, 'fork_at', 'loro.fork_at')
        text = function(text, 'snapshot', 'loro.export_snapshot')
        return text
    edit('crates/core/src/doc.rs', doc)
    def sync(text):
        for name, label in [('sha256_hex','hash.sha256'), ('file_is_recent','stat.file_is_recent'),
                            ('read_cold','sync.read_cold'), ('export_updates','sync.export_updates'),
                            ('import_updates_batch','sync.import_batch'), ('import_hot','sync.import_hot'),
                            ('import_cold','sync.import_cold')]:
            text = function(text, name, label)
        for mode, label in [('Snapshot','loro.export_snapshot'), ('updates(&since)','loro.export_updates')]:
            old = f'            st.doc.doc.export(ExportMode::{mode})?'
            new = '            {\n                ' + guard(label).replace('\n        ', '\n                ') + f'                st.doc.doc.export(ExportMode::{mode})?\n            }}'
            text = replace(text, old, new)
        text = replace(text, '        Ok(bytes)\n    }\n\n    /// Imports one page', '''        #[cfg(feature = "storage-metrics")]
        crate::storage_metrics::add_bytes(if since.is_empty() {
            "loro.export_snapshot"
        } else { "loro.export_updates" }, bytes.len() as u64);
        Ok(bytes)
    }

    /// Imports one page''')
        return text
    edit('crates/core/src/sync.rs', sync)
    def vault(text):
        for name, label in [('flush_lock','wait.flush_lock'), ('stat_page_file','stat.page_file'),
                            ('read_page_file','disk.read_page_file'), ('read_doc_tracked','vault.read_doc_tracked'),
                            ('write_page_doc','disk.write_page_doc')]:
            text = function(text, name, label)
        text = replace(text, '        Ok(fs::read(self.page_file_path(id, kind))?)', '''        let bytes = fs::read(self.page_file_path(id, kind))?;
        #[cfg(feature = "storage-metrics")]
        crate::storage_metrics::add_bytes("disk.read_page_file", bytes.len() as u64);
        Ok(bytes)''')
        return text
    edit('crates/core/src/vault.rs', vault)
    # Sync's only changes relative to the baseline are telemetry and bench modes.
    for relative in ['crates/sync/Cargo.toml', 'crates/sync/src/lib.rs', 'crates/sync/src/hub.rs',
                     'crates/sync/src/client.rs', 'crates/sync/examples/sync_bench.rs']:
        shutil.copyfile(candidate / relative, master / relative)
    print(f'Feature-only overlay ready: {master}')


if __name__ == '__main__':
    main()

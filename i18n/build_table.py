#!/usr/bin/env python3
"""Builds the Chinese table (``desktop/src/lang/zh.rs``) from the translators' ``part*.zh.txt``.

Each translator's file is ``<English source> <TAB> <Chinese>``, one line per string it was given.
This checks every line against the very strings the app draws (``strings.json``, which the same
scan produced), and writes the table the app looks words up in.

    python3 build_table.py

A line whose English isn't one of the app's strings is reported and dropped: a translator working
from a stale list must not put words in the app that nothing asks for.
"""
import json
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.join(HERE, '..', 'procraft', 'desktop', 'src')


def rust_str(text):
    """`text` as a Rust string literal's insides."""
    out = []
    for c in text:
        if c == '\\':
            out.append('\\\\')
        elif c == '"':
            out.append('\\"')
        elif c == '\n':
            out.append('\\n')
        elif c == '\r':
            out.append('\\r')
        elif c == '\t':
            out.append('\\t')
        else:
            out.append(c)
    return ''.join(out)


def main():
    strings = json.load(open(os.path.join(HERE, 'strings.json'), encoding='utf-8'))
    wanted = set(strings)

    table = {}
    problems = []
    for name in sorted(os.listdir(HERE)):
        if not re.fullmatch(r'part\d+\.zh\.txt', name):
            continue
        path = os.path.join(HERE, name)
        with open(path, encoding='utf-8') as f:
            body = f.read()
        for at, line in enumerate(body.split('\n'), 1):
            if not line.strip():
                continue
            if '\t' not in line:
                problems.append(f'{name}:{at}: no tab: {line[:60]!r}')
                continue
            source, said = line.split('\t', 1)
            said = said.rstrip('\r')
            if source not in wanted:
                problems.append(f'{name}:{at}: not an app string: {source[:60]!r}')
                continue
            # Untranslated (the translator kept the English) is fine and is left out: the lookup
            # falls back to the English anyway.
            if said and said != source:
                table[source] = said

    missing = [s for s in wanted if s not in table]
    out = os.path.join(SRC, 'lang', 'zh.rs')
    with open(out, 'w', encoding='utf-8') as f:
        f.write('//! The interface in Chinese: `(English as the code writes it, its Chinese)`.\n')
        f.write('//!\n')
        f.write('//! Built from the translators\' files by `i18n/build_table.py`; a string that isn\'t\n')
        f.write('//! here is shown in English (see `lang::tr`).\n\n')
        f.write('/// The Chinese for the English the interface writes, sorted so a lookup can bisect.\n')
        f.write('static TABLE: [(&str, &str); %d] = [\n' % len(table))
        for source in sorted(table):
            f.write('    ("%s", "%s"),\n' % (rust_str(source), rust_str(table[source])))
        f.write('];\n\n')
        f.write('''/// The Chinese `text` is shown as, if the table has it.
pub fn zh(text: &str) -> Option<&'static str> {
    TABLE
        .binary_search_by(|(english, _)| (*english).cmp(text))
        .ok()
        .map(|at| TABLE[at].1)
}

/// The Chinese for words the code built itself (`format!` and the like): the table's own entry
/// when the whole of what was built is one of its sentences, else none (the words are the user's
/// own, or pieces already translated).
pub fn zh_dynamic(text: &str) -> Option<String> {
    zh(text).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_is_found_and_sorted() {
        let mut last = "";
        for (english, chinese) in TABLE {
            assert!(english >= last, "the table is not sorted at {english:?}");
            last = english;
            assert_eq!(zh(english), Some(chinese), "{english:?}");
            assert!(!chinese.is_empty(), "{english:?} says nothing");
        }
    }

    #[test]
    fn something_is_translated_at_all() {
        assert!(TABLE.len() > 500, "only {} strings translated", TABLE.len());
        assert_eq!(zh("Do not translate this"), None);
    }
}
''')
    print(f'{len(table)} strings in the table -> {out}')
    if missing:
        print(f'{len(missing)} strings have no Chinese yet (they show in English):')
        for s in missing[:15]:
            print('   ', s[:76])
        if len(missing) > 15:
            print(f'    ... and {len(missing) - 15} more')
    if problems:
        print(f'{len(problems)} problems:')
        for p in problems[:15]:
            print('   ', p)


if __name__ == '__main__':
    main()

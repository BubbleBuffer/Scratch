// The "Bake-off showcase" page (the Bake-off spec in the internal wiki; wiki/prototypes/flutter.md): one block of
// every built-in kind, some inline styles and a nested list. Created with the
// bench seed (SEED_BENCH) and by the tests; not part of a normal vault.

import '../core/models.dart';
import '../core/vault.dart';

const showcaseTitle = 'Bake-off showcase';

class _B {
  const _B(this.id, this.kind, this.text, {this.parent, this.marks = const [], this.checked = false});
  final String id;
  final String kind;
  final String text;
  final String? parent;
  final List<(String, BMarkKind, String)> marks; // (substring, kind, url)
  final bool checked;
}

const _blocks = <_B>[
  _B('sc01', Kinds.paragraph,
      'This page has one block of every kind the prototype supports, with bold, italic, strikethrough, inline code and a link.',
      marks: [
        ('bold', BMarkKind.bold, ''),
        ('italic', BMarkKind.italic, ''),
        ('strikethrough', BMarkKind.strike, ''),
        ('inline code', BMarkKind.code, ''),
        ('a link', BMarkKind.link, 'https://example.com'),
      ]),
  _B('sc02', Kinds.heading1, 'Heading 1'),
  _B('sc03', Kinds.paragraph, 'Plain paragraph text under a large heading. Type / for commands.'),
  _B('sc04', Kinds.heading2, 'Heading 2'),
  _B('sc05', Kinds.bulleted, 'Bulleted list item'),
  _B('sc06', Kinds.bulleted, 'Nested item', parent: 'sc05'),
  _B('sc07', Kinds.bulleted, 'Deeper item', parent: 'sc06'),
  _B('sc08', Kinds.bulleted, 'Another top-level item'),
  _B('sc09', Kinds.heading3, 'Heading 3'),
  _B('sc10', Kinds.numbered, 'First step'),
  _B('sc11', Kinds.numbered, 'Second step'),
  _B('sc12', Kinds.numbered, 'A nested step', parent: 'sc11'),
  _B('sc13', Kinds.numbered, 'Third step'),
  _B('sc14', Kinds.todo, 'Seed the vault', checked: true),
  _B('sc15', Kinds.todo, 'Compare both prototypes'),
  _B('sc16', Kinds.quote, 'Simple things should be simple, complex things should be possible.'),
  _B('sc17', Kinds.code, 'fn main() {\n    println!("Hello, vault");\n}'),
  _B('sc18', Kinds.divider, ''),
  _B('sc19', Kinds.paragraph, 'The end of the showcase.'),
];

List<BMark> _marks(String text, List<(String, BMarkKind, String)> spec) => [
      for (final (sub, kind, url) in spec)
        if (text.contains(sub))
          BMark(start: text.indexOf(sub), end: text.indexOf(sub) + sub.length, kind: kind, url: url),
    ];

/// The ops that fill an empty page with the showcase content.
List<BOp> showcaseOps() {
  final lastChild = <String?, String?>{};
  final ops = <BOp>[];
  for (final b in _blocks) {
    ops.add(BOp(
      op: OpKind.insert,
      id: b.id,
      parent: b.parent,
      after: lastChild[b.parent],
      kind: b.kind,
      text: b.text,
      marks: _marks(b.text, b.marks),
      checked: b.checked,
    ));
    lastChild[b.parent] = b.id;
  }
  return ops;
}

Future<String> createShowcase(Vault vault, {String? parent}) async {
  final id = await vault.createPage(parent: parent, title: showcaseTitle);
  await vault.apply(id, showcaseOps());
  return id;
}

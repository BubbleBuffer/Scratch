// Block codecs: each kind's markdown, clipboard and plain-text codecs, the
// explicit generic fallback, and the contract that a new kind cannot be
// missing its codecs without a test failing.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/blocks/blocks.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/clipboard.dart';
import 'package:notes_app/editor/model.dart';
import 'package:notes_app/registry/block_codecs.dart';
import 'package:notes_app/registry/block_registry.dart';

/// Core kinds with legacy codecs. New registrations with their own codecs
/// are covered by their registration contract below.
const _coreKinds = [
  Kinds.paragraph,
  Kinds.heading1,
  Kinds.heading2,
  Kinds.heading3,
  Kinds.bulleted,
  Kinds.numbered,
  Kinds.todo,
  Kinds.quote,
  Kinds.code,
  Kinds.divider,
  Kinds.linkToPage,
  Kinds.image,
  Kinds.file,
  Kinds.bookmark,
  Kinds.toggle,
  Kinds.toggleHeading1,
  Kinds.toggleHeading2,
  Kinds.toggleHeading3,
  Kinds.callout,
  Kinds.columnList,
  Kinds.column,
  Kinds.toc,
  Kinds.chatRef,
];

int _ids = 0;

Node _block(
  String kind, {
  String text = '',
  String? target,
  bool checked = false,
  int number = 1,
  List<BMark> marks = const [],
  List<Node> children = const [],
}) {
  final n = Node(
    id: 'b${_ids++}',
    kind: kind,
    text: text,
    target: target,
    checked: checked,
    marks: List.of(marks),
  )..number = number;
  for (final c in children) {
    c.parent = n;
    n.children.add(c);
  }
  return n;
}

/// [tops] as markdown through the global registry (its legacy codecs unless
/// something was registered).
String _md(List<Node> tops) => nodesToMarkdown(tops);

void main() {
  group('legacy codecs (what the clipboard always wrote)', () {
    final cases = <(String, Node, String)>[
      ('paragraph', _block(Kinds.paragraph, text: 'plain'), 'plain'),
      ('heading1', _block(Kinds.heading1, text: 'H'), '# H'),
      ('toggle heading1', _block(Kinds.toggleHeading1, text: 'H'), '# H'),
      ('heading2', _block(Kinds.heading2, text: 'H'), '## H'),
      ('toggle heading2', _block(Kinds.toggleHeading2, text: 'H'), '## H'),
      ('heading3', _block(Kinds.heading3, text: 'H'), '### H'),
      ('toggle heading3', _block(Kinds.toggleHeading3, text: 'H'), '### H'),
      ('bulleted', _block(Kinds.bulleted, text: 'b'), '- b'),
      ('numbered', _block(Kinds.numbered, text: 'n', number: 3), '3. n'),
      ('todo checked', _block(Kinds.todo, text: 't', checked: true), '- [x] t'),
      ('todo open', _block(Kinds.todo, text: 't'), '- [ ] t'),
      ('quote', _block(Kinds.quote, text: 'q'), '> q'),
      ('toggle', _block(Kinds.toggle, text: 'T'), '▸ T'),
      ('callout default icon', _block(Kinds.callout, text: 'c'), '> 💡 c'),
      (
        'callout icon',
        _block(Kinds.callout, text: 'c', target: '🔥'),
        '> 🔥 c',
      ),
      ('code', _block(Kinds.code, text: 'a\nb'), '```\na\nb\n```'),
      ('divider', _block(Kinds.divider), '---'),
      (
        'image',
        _block(Kinds.image, text: 'cap', target: 'abc'),
        '![cap](attachment:abc)',
      ),
      ('file', _block(Kinds.file, target: 'f1'), '[File](attachment:f1)'),
      (
        'file named',
        _block(Kinds.file, text: 'doc', target: 'f1'),
        '[doc](attachment:f1)',
      ),
      (
        'bookmark',
        _block(Kinds.bookmark, target: 'https://x.test'),
        '[https://x.test](https://x.test)',
      ),
      (
        'bookmark captioned',
        _block(Kinds.bookmark, text: 'Site', target: 'https://x.test'),
        '[Site](https://x.test)',
      ),
      (
        'link to page',
        _block(Kinds.linkToPage, target: 'p1'),
        '[Page](page:p1)',
      ),
      (
        'inline marks',
        _block(
          Kinds.paragraph,
          text: 'bold it',
          marks: [BMark(start: 0, end: 4, kind: BMarkKind.bold, url: '')],
        ),
        '**bold** it',
      ),
    ];
    for (final (name, node, expected) in cases) {
      test('$name writes "$expected"', () {
        expect(_md([node]), expected);
      });
    }

    test('columns and a table of contents flatten to their contents', () {
      final columns = _block(
        Kinds.columnList,
        children: [
          _block(Kinds.column, children: [_block(Kinds.paragraph, text: 'a')]),
          _block(Kinds.column, children: [_block(Kinds.paragraph, text: 'b')]),
        ],
      );
      expect(_md([columns]), 'a\nb');
      expect(_md([_block(Kinds.toc)]), '');
    });

    test(
      'children indent under their parent, including under toggles and code',
      () {
        final tree = _block(
          Kinds.bulleted,
          text: 'one',
          children: [
            _block(
              Kinds.bulleted,
              text: 'two',
              children: [_block(Kinds.numbered, text: 'three')],
            ),
            _block(Kinds.code, text: 'x', children: []),
          ],
        );
        expect(
          _md([tree]),
          '- one\n    - two\n        1. three\n    ```\n    x\n    ```',
        );
        final toggle = _block(
          Kinds.toggle,
          text: 'T',
          children: [_block(Kinds.paragraph, text: 'inside')],
        );
        expect(_md([toggle]), '▸ T\n    inside');
      },
    );
  });

  group('plain text codecs', () {
    test('one line per block with text, no markers, children included', () {
      final tops = [
        _block(
          Kinds.bulleted,
          text: 'one',
          children: [_block(Kinds.image, text: 'cap', target: 'x')],
        ),
        _block(Kinds.divider),
        _block(Kinds.heading1, text: 'Two'),
      ];
      expect(nodesToPlainText(tops), 'one\ncap\nTwo');
    });

    test('code keeps its raw text', () {
      expect(
        nodesToPlainText([_block(Kinds.code, text: 'let x = 1;')]),
        'let x = 1;',
      );
    });
  });

  group('clipboard codec', () {
    test('defaults to the markdown the clipboard always carried', () {
      final tops = [
        _block(
          Kinds.heading2,
          text: 'H',
          children: [_block(Kinds.todo, text: 't', checked: true)],
        ),
      ];
      expect(clipboardText(tops), nodesToMarkdown(tops));
      expect(clipboardText(tops), '## H\n    - [x] t');
    });
  });

  group('generic fallback', () {
    test('an unknown kind writes its text as a line, children indented', () {
      final node = _block(
        'mystery',
        text: 'hi',
        children: [_block(Kinds.paragraph, text: 'kid')],
      );
      expect(_md([node]), 'hi\n    kid');
      expect(nodesToPlainText([node]), 'hi\nkid');
    });

    test('the fallback is flagged generic, and only chat references use it among core kinds', () {
      expect(genericBlockCodecs.generic, isTrue);
      final generic = [
        for (final k in _coreKinds)
          if (legacyBlockCodecs(k)!.generic) k,
      ];
      expect(generic, [Kinds.chatRef]);
    });
  });

  group('contract', () {
    // Kinds.table is not in _coreKinds: the legacy table would need an entry
    // for it, and that table is outside this change. Its codecs come from its
    // own registration instead, checked here without touching the legacy cases.
    test('the table kind has codecs from its registration, not the generic fallback', () {
      final reg = BlockRegistry();
      registerBuiltinBlocks(reg);
      expect(legacyBlockCodecs(Kinds.table), isNull);
      expect(reg.codecsOf(Kinds.table).generic, isFalse);
      expect(reg.incompleteCodecKinds, isNot(contains(Kinds.table)));
      const grid = '| a | b |\n| - | - |\n| 1 | 2 |';
      expect(
        nodesToMarkdown([_block(Kinds.table, text: grid)], registry: reg),
        grid,
      );
    });

    test(
      'every legacy core kind has a codec, and the legacy list is complete',
      () {
        expect(legacyCodecKinds.toSet(), _coreKinds.toSet());
        for (final k in _coreKinds) {
          expect(legacyBlockCodecs(k), isNotNull, reason: '$k has no codecs');
        }
      },
    );

    test('a registered kind with no codecs of its own and no legacy entry is reported', () {
      final reg = BlockRegistry()
        ..register(const BlockType(kind: 'future_kind', label: 'Future'));
      expect(reg.incompleteCodecKinds, ['future_kind']);
      final codecs = reg.codecsOf('future_kind');
      expect(codecs.generic, isTrue);
      expect(
        nodesToMarkdown([_block('future_kind', text: 'x')], registry: reg),
        'x',
      );
    });

    test(
      'a registration without codecs of a legacy kind uses the legacy codecs',
      () {
        final reg = BlockRegistry()
          ..register(const BlockType(kind: Kinds.heading1, label: 'Heading 1'));
        expect(reg.incompleteCodecKinds, isEmpty);
        expect(reg.codecsOf(Kinds.heading1).generic, isFalse);
        expect(
          nodesToMarkdown([_block(Kinds.heading1, text: 'H')], registry: reg),
          '# H',
        );
      },
    );

    test(
      'a new kind with declared codecs is complete and writes through them',
      () {
        final reg = BlockRegistry()
          ..register(
            BlockType(
              kind: 'future_kind',
              label: 'Future',
              codecs: BlockCodecs(
                markdown: MarkdownCodec((n, t) => ['!! $t']),
                clipboard: const ClipboardCodec.viaMarkdown(),
                plainText: PlainTextCodec((n) => 'plain ${n.text}'),
              ),
            ),
          );
        expect(reg.incompleteCodecKinds, isEmpty);
        final tops = [_block('future_kind', text: 'x')];
        expect(nodesToMarkdown(tops, registry: reg), '!! x');
        expect(clipboardText(tops, registry: reg), '!! x');
        expect(nodesToPlainText(tops, registry: reg), 'plain x');
        expect(reg.codecsOf('future_kind').generic, isFalse);
      },
    );

    test('a clipboard codec of its own changes what copy writes, not the markdown export', () {
      final reg = BlockRegistry()
        ..register(
          BlockType(
            kind: 'future_kind',
            label: 'Future',
            codecs: BlockCodecs(
              markdown: MarkdownCodec((n, t) => ['!! $t']),
              clipboard: ClipboardCodec.custom(
                MarkdownCodec((n, t) => ['copied: $t']),
              ),
              plainText: PlainTextCodec((n) => n.text),
            ),
          ),
        );
      final tops = [_block('future_kind', text: 'x')];
      expect(nodesToMarkdown(tops, registry: reg), '!! x');
      expect(clipboardText(tops, registry: reg), 'copied: x');
    });

    test('a Rust codec reference is declared (not generic) but throws until a Dart resolver is wired', () {
      const id = 'core.mdio.blocks_markdown';
      final reg = BlockRegistry()
        ..register(
          BlockType(
            kind: 'rust_kind',
            label: 'Rust',
            codecs: BlockCodecs(
              markdown: const MarkdownCodec.rust(id),
              clipboard: const ClipboardCodec.viaMarkdown(),
              plainText: PlainTextCodec((n) => n.text),
            ),
          ),
        );
      expect(reg.incompleteCodecKinds, isEmpty);
      expect(reg.codecsOf('rust_kind').markdown.rustId, id);
      expect(reg.codecsOf('rust_kind').generic, isFalse);
      expect(
        () => nodesToMarkdown([_block('rust_kind', text: 'x')], registry: reg),
        throwsStateError,
      );
    });
  });
}

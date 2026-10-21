// Text codecs for block kinds. Each block type can carry a [BlockCodecs]: how
// the block writes as markdown, what copying it puts on the system clipboard,
// and its plain text. The registry resolves a kind's codecs (its registration,
// else the legacy table below, else the explicit generic fallback), and
// clipboard.dart walks the tree through them. Parsing markdown stays one
// shared parser (markdownToNodes) for every kind.

import 'dart:convert';

import '../core/models.dart';
import '../editor/model.dart';

/// The markdown lines of one block, without indentation (the writer indents
/// them). [inline] is the block's text with its marks as inline markdown.
typedef MarkdownLines = List<String> Function(Node n, String inline);

/// How a block writes as markdown.
class MarkdownCodec {
  /// Lines from [f]. A [flattens] block writes no line of its own: its
  /// children are written at its level (columns, a table of contents).
  const MarkdownCodec(MarkdownLines f, {this.flattens = false})
    : lines = f,
      rustId = null;

  /// A codec that stays in Rust: the core's markdown writer
  /// (`crates/core/src/mdio.rs`). A registration references it by [id]
  /// rather than duplicating it. Dart has no synchronous call into the core,
  /// so writing such a block throws until a synchronous resolver is wired.
  const MarkdownCodec.rust(String id)
    : lines = null,
      flattens = false,
      rustId = id;

  final MarkdownLines? lines;
  final bool flattens;
  final String? rustId;

  List<String> write(Node n, String inline) {
    final f = lines;
    if (f != null) return f(n, inline);
    throw StateError(
      'Markdown for "${n.kind}" is the Rust codec "$rustId", which has no synchronous Dart resolver',
    );
  }
}

/// What copying a block puts on the system clipboard. The default
/// ([ClipboardCodec.viaMarkdown]) is the block's markdown, which is what the
/// clipboard has always carried. Paste reads clipboard text with the shared
/// markdown parser, whatever the kind.
class ClipboardCodec {
  const ClipboardCodec.viaMarkdown() : lines = null;

  const ClipboardCodec.custom(MarkdownCodec codec) : lines = codec;

  /// Null: copy the block's markdown codec.
  final MarkdownCodec? lines;
}

/// What a block is as plain text: one line, or empty for none.
class PlainTextCodec {
  const PlainTextCodec(this.line);

  final String Function(Node n) line;
}

/// The three text codecs of one block kind. All three are required, so a
/// registration cannot leave one out silently.
class BlockCodecs {
  const BlockCodecs({
    required this.markdown,
    required this.clipboard,
    required this.plainText,
    this.generic = false,
  });

  final MarkdownCodec markdown;
  final ClipboardCodec clipboard;
  final PlainTextCodec plainText;

  /// The explicit fallback for a kind nothing declares. Contract tests list
  /// which kinds resolve to it, so a new kind cannot be generic by accident.
  final bool generic;
}

// ---------------------------------------------------------------------------
// The legacy table: the codecs the Dart clipboard always had, as data. Kinds
// registered without codecs of their own resolve here.

String _plainTextOf(Node n) => n.text;

final _plainText = PlainTextCodec(_plainTextOf);

const _viaMarkdown = ClipboardCodec.viaMarkdown();

BlockCodecs _textLine(String Function(Node n, String t) line) => BlockCodecs(
  markdown: MarkdownCodec((n, t) => [line(n, t)]),
  clipboard: _viaMarkdown,
  plainText: _plainText,
);

BlockCodecs _flattening() => BlockCodecs(
  markdown: MarkdownCodec((n, t) => const [], flattens: true),
  clipboard: _viaMarkdown,
  plainText: _plainText,
);

/// The explicit fallback for a kind nothing declares: its text as one line,
/// its children indented under it. This is what the markdown writer always
/// did for kinds it did not know.
final genericBlockCodecs = BlockCodecs(
  markdown: MarkdownCodec((n, t) => [t]),
  clipboard: _viaMarkdown,
  plainText: _plainText,
  generic: true,
);

/// Lossless generic Markdown, shared with the core's block annotation format.
/// The visible fallback can be computed at export; it need not be block text.
List<String> annotatedBlockMarkdown(Node n, String fallback) {
  final lines = fallback.split('\n');
  final data = utf8.encode(
    jsonEncode({
      'kind': n.kind,
      'target': n.target,
      'lines': lines.length,
      'checked': n.checked,
    }),
  );
  final hex = data.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
  final meta = [
    'block=$hex',
    if (n.color != null && RegExp(r'^[^\s<>]+$').hasMatch(n.color!))
      'color=${n.color}',
    if (n.width != null && n.width! > 0 && n.width! < 100) 'width=${n.width}',
  ].join(' ');
  return [
    '> ${lines.first.isEmpty ? '' : '${lines.first} '}<!-- $meta -->',
    for (final line in lines.skip(1)) '> $line',
  ];
}

final _paragraph = _textLine((n, t) => t);
final _heading1 = _textLine((n, t) => '# $t');
final _heading2 = _textLine((n, t) => '## $t');
final _heading3 = _textLine((n, t) => '### $t');
final _bulleted = _textLine((n, t) => '- $t');
final _numbered = _textLine((n, t) => '${n.number}. $t');
final _todo = _textLine((n, t) => '- [${n.checked ? 'x' : ' '}] $t');
final _quote = _textLine((n, t) => '> $t');
final _toggle = _textLine((n, t) => '▸ $t');
final _callout = _textLine((n, t) => '> ${n.target ?? '💡'} $t');
final _divider = _textLine((n, t) => '---');
final _image = _textLine(
  (n, t) => '![${n.text}](attachment:${n.target ?? ''})',
);
final _file = _textLine(
  (n, t) =>
      '[${n.text.isEmpty ? 'File' : n.text}](attachment:${n.target ?? ''})',
);
final _bookmark = _textLine(
  (n, t) =>
      '[${n.text.isEmpty ? (n.target ?? '') : n.text}](${n.target ?? ''})',
);
final _linkToPage = _textLine(
  (n, t) => '[${n.text.isEmpty ? 'Page' : n.text}](page:${n.target ?? ''})',
);
final _code = BlockCodecs(
  // The code's own text, unmarked and unindented; the writer indents each line.
  markdown: MarkdownCodec((n, t) => ['```', ...n.text.split('\n'), '```']),
  clipboard: _viaMarkdown,
  plainText: _plainText,
);
final _flatten = _flattening();

final _legacy = <String, BlockCodecs>{
  Kinds.paragraph: _paragraph,
  Kinds.heading1: _heading1,
  Kinds.toggleHeading1: _heading1,
  Kinds.heading2: _heading2,
  Kinds.toggleHeading2: _heading2,
  Kinds.heading3: _heading3,
  Kinds.toggleHeading3: _heading3,
  Kinds.bulleted: _bulleted,
  Kinds.numbered: _numbered,
  Kinds.todo: _todo,
  Kinds.quote: _quote,
  Kinds.toggle: _toggle,
  Kinds.callout: _callout,
  Kinds.code: _code,
  Kinds.divider: _divider,
  Kinds.image: _image,
  Kinds.file: _file,
  Kinds.bookmark: _bookmark,
  Kinds.linkToPage: _linkToPage,
  Kinds.columnList: _flatten,
  Kinds.column: _flatten,
  Kinds.toc: _flatten,
  // Added by the chat-reference add-on; it writes its text generically until
  // that add-on declares its own codecs.
  Kinds.chatRef: genericBlockCodecs,
};

/// The legacy codecs of a core kind, or null for a kind outside the table.
/// Built-in registrations can pass this as their `codecs`.
BlockCodecs? legacyBlockCodecs(String kind) => _legacy[kind];

/// The kinds the legacy table covers (contract tests iterate these).
Iterable<String> get legacyCodecKinds => _legacy.keys;

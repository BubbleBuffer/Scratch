import 'navigation.dart';
import '../../services/addon_services.dart';
// Assistant text as read-only blocks. The vault's own markdown reader
// (`agentMarkdownBlocks` in the bridge) parses it, so chat markdown reads
// like imported notes: headings, bullet and numbered lists nested by
// indent, to-dos, quotes, code fences, dividers, and bold, italic, strike,
// inline code and links inside the text.
//
// - http(s) and mail links open outside the app ([openWebLink]).
// - Page links (`[[Title]]`, or `[text](target)` whose target or text names a
//   page by id or title) open the page.
// - User messages are not parsed: they stay plain text.

import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';

import '../../core/page_link.dart';
import '../../core/paths.dart' show AppLog;
import '../../src/rust/api/agent.dart' show agentMarkdownBlocks;
import '../../src/rust/api/types.dart';
import '../../src/rust/api/vault.dart' show NotesVault;
import '../../state/app_state.dart';
import '../../ui/ui.dart';

/// Opens a web or mail link outside the app. Tests replace it.
Future<void> Function(String url) openWebLink = _openWithSystem;

/// The desktop's default handler for [url] (the agent runs on Windows and
/// Linux). No shell is involved, so the URL is passed as one argument.
Future<void> _openWithSystem(String url) async {
  try {
    if (Platform.isWindows) {
      await Process.start('rundll32', [
        'url.dll,FileProtocolHandler',
        url,
      ], mode: ProcessStartMode.detached);
    } else if (Platform.isLinux) {
      await Process.start('xdg-open', [url], mode: ProcessStartMode.detached);
    } else if (Platform.isMacOS) {
      await Process.start('open', [url], mode: ProcessStartMode.detached);
    }
  } catch (e) {
    AppLog.info('open link: $e');
  }
}

/// Whether [url] opens outside the app.
bool isWebLink(String url) {
  final u = url.trim().toLowerCase();
  return u.startsWith('http://') ||
      u.startsWith('https://') ||
      u.startsWith('mailto:');
}

final _parsed = Expando<Map<String, List<FlatBlock>>>();
const _pageUrlPrefix = 'https://notes-agent-page.invalid/';

/// [text] as display blocks, parsed once per text and vault (a live reply
/// is parsed again only when a paragraph arrives).
List<FlatBlock> markdownBlocks(NotesVault vault, String text) {
  final cache = _parsed[vault] ??= <String, List<FlatBlock>>{};
  final hit = cache.remove(text);
  if (hit != null) return cache[text] = hit;
  // Prevent the bridge's legacy label fallback from overriding an explicit
  // page id when the label happens to name a different page.
  final pageUrls = <String, String>{};
  final input = text.replaceAllMapped(RegExp(r'\]\(page:([^\)]+)\)'), (match) {
    final url = '$_pageUrlPrefix${Uri.encodeComponent(match[1]!)}';
    pageUrls[url] = 'page:${match[1]}';
    return ']($url)';
  });
  FlatBlock restoreBlock(FlatBlock b) {
    final offsets = List<int>.filled(b.text.length + 1, 0);
    final restored = StringBuffer();
    var at = 0, length = 0;
    while (at < b.text.length) {
      final replacement = pageUrls.entries
          .where((e) => b.text.startsWith(e.key, at))
          .firstOrNull;
      if (replacement == null) {
        offsets[at] = length;
        restored.write(b.text[at++]);
        length++;
      } else {
        for (var i = 0; i < replacement.key.length; i++) {
          offsets[at + i] = length + i.clamp(0, replacement.value.length);
        }
        restored.write(replacement.value);
        at += replacement.key.length;
        length += replacement.value.length;
      }
    }
    offsets[at] = length;
    BMark restoreMark(BMark m) {
      final pageUrl = pageUrls.containsKey(m.url);
      final page = pageUrl || m.url.startsWith('page:');
      return BMark(
        start: offsets[m.start.clamp(0, b.text.length)],
        end: offsets[m.end.clamp(0, b.text.length)],
        kind: page ? BMarkKind.pageLink : m.kind,
        url: pageUrl
            ? Uri.decodeComponent(m.url.substring(_pageUrlPrefix.length))
            : page
            ? m.url.substring(5)
            : m.url,
      );
    }

    return FlatBlock(
      id: b.id,
      kind: b.kind,
      text: restored.toString(),
      checked: b.checked,
      depth: b.depth,
      target: b.target,
      color: b.color,
      width: b.width,
      marks: [for (final m in b.marks) restoreMark(m)],
    );
  }

  final blocks = [
    for (final b in agentMarkdownBlocks(vault: vault, text: input))
      restoreBlock(b),
  ];
  if (cache.length >= 200) cache.remove(cache.keys.first);
  return cache[text] = blocks;
}

/// Assistant text rendered as markdown.
class AgentMarkdown extends StatelessWidget {
  const AgentMarkdown(this.text, {super.key});
  final String text;

  @override
  Widget build(BuildContext context) {
    final blocks = markdownBlocks(context.app.vault.raw, text);
    final children = <Widget>[];
    // Numbered items count per depth; anything else at that depth or
    // shallower restarts the count.
    final numbers = <int, int>{};
    String? previous;
    for (final (i, b) in blocks.indexed) {
      numbers.removeWhere(
        (depth, _) =>
            depth > b.depth || (depth == b.depth && b.kind != 'numbered'),
      );
      final n = b.kind == 'numbered'
          ? numbers[b.depth] = (numbers[b.depth] ?? 0) + 1
          : 0;
      if (i > 0) children.add(SizedBox(height: _gap(previous!, b.kind)));
      children.add(_BlockView(b, number: n, key: ValueKey('md-$i')));
      previous = b.kind;
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: children,
    );
  }

  static bool _isItem(String kind) =>
      kind == 'bulleted' || kind == 'numbered' || kind == 'todo';

  /// List items sit close together; other blocks get paragraph spacing.
  static double _gap(String before, String kind) {
    if (_isItem(before) && _isItem(kind)) return 4;
    if (kind.startsWith('heading')) return 14;
    return 10;
  }
}

class _BlockView extends StatelessWidget {
  const _BlockView(this.block, {super.key, this.number = 0});
  final FlatBlock block;
  final int number;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final type = context.type;
    final b = block;
    final indent = b.depth * 22.0;
    Widget text(TextStyle style) => _MarkedText(b.text, b.marks, style);
    final body = ts(type.body, c.textPrimary);
    final Widget view = switch (b.kind) {
      'heading1' || 'toggleheading1' => text(ts(type.h3, c.textPrimary)),
      'heading2' ||
      'toggleheading2' => text(ts(const TypeSpec(18, 25, 600), c.textPrimary)),
      'heading3' ||
      'toggleheading3' => text(ts(const TypeSpec(16, 24, 600), c.textPrimary)),
      'code' => CodeBox(b.text),
      'divider' => Padding(
        padding: const EdgeInsets.symmetric(vertical: 6),
        child: Container(height: 1, color: c.divider),
      ),
      'quote' => Container(
        padding: const EdgeInsets.only(left: 14),
        decoration: BoxDecoration(
          border: Border(left: BorderSide(color: c.borderStrong, width: 3)),
        ),
        child: text(ts(type.body, c.textSecondary)),
      ),
      'bulleted' || 'numbered' || 'todo' => Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 22,
            child: Text(
              switch (b.kind) {
                'numbered' => '$number.',
                'todo' => b.checked ? '☑' : '☐',
                _ => b.depth.isEven ? '•' : '◦',
              },
              key: ValueKey('md-marker-${b.kind}'),
              style: ts(type.body, c.textSecondary),
            ),
          ),
          Expanded(
            child: text(
              b.kind == 'todo' && b.checked
                  ? body.copyWith(color: c.textTertiary)
                  : body,
            ),
          ),
        ],
      ),
      _ => text(body),
    };
    return Padding(
      padding: EdgeInsets.only(left: indent),
      child: view,
    );
  }
}

/// Text with its marks; link and page-link ranges are tappable.
class _MarkedText extends StatefulWidget {
  const _MarkedText(this.text, this.marks, this.style);
  final String text;
  final List<BMark> marks;
  final TextStyle style;

  @override
  State<_MarkedText> createState() => _MarkedTextState();
}

class _MarkedTextState extends State<_MarkedText> {
  final _taps = <TapGestureRecognizer>[];

  @override
  void dispose() {
    for (final r in _taps) {
      r.dispose();
    }
    super.dispose();
  }

  BMark? _linkAt(int offset) {
    for (final m in widget.marks) {
      if ((m.kind == BMarkKind.link || m.kind == BMarkKind.pageLink) &&
          m.start <= offset &&
          offset < m.end) {
        return m;
      }
    }
    return null;
  }

  void _open(BMark m) {
    final label = widget.text.substring(
      m.start.clamp(0, widget.text.length),
      m.end.clamp(0, widget.text.length),
    );
    final pageLink = PageLink.parse(m.url);
    if (m.kind == BMarkKind.pageLink) {
      openAgentPage(context, m.url, label);
    } else if (pageLink != null) {
      openAgentPageLink(context, pageLink);
    } else if (isWebLink(m.url)) {
      openWebLink(m.url.trim());
    }
  }

  @override
  Widget build(BuildContext context) {
    for (final r in _taps) {
      r.dispose();
    }
    _taps.clear();
    final c = context.colors;
    final styled = markedSpan(widget.text, widget.marks, c, widget.style);
    final inline = <InlineSpan>[];
    var at = 0;
    for (final child in styled.children ?? [styled]) {
      if (child is! TextSpan) {
        inline.add(child);
        continue;
      }
      final segment = child.text ?? '';
      final link = segment.isEmpty ? null : _linkAt(at);
      at += segment.length;
      if (link == null) {
        inline.add(
          child.style?.fontFamily == monoFamily
              ? TextSpan(
                  text: segment,
                  style: child.style!.copyWith(backgroundColor: c.inlineCodeBg),
                )
              : child,
        );
        continue;
      }
      final tap = TapGestureRecognizer()..onTap = () => _open(link);
      _taps.add(tap);
      inline.add(
        TextSpan(
          text: segment,
          style: child.style,
          recognizer: tap,
          mouseCursor: SystemMouseCursors.click,
        ),
      );
    }
    return Text.rich(TextSpan(style: widget.style, children: inline));
  }
}

/// Monospace text in the code block's box; long lines scroll sideways.
class CodeBox extends StatelessWidget {
  const CodeBox(this.text, {super.key, this.color, this.textKey});
  final String text;
  final Color? color;
  final Key? textKey;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      decoration: BoxDecoration(
        color: c.bgSecondary,
        borderRadius: BorderRadius.circular(Radii.sm),
      ),
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
      child: LayoutBuilder(
        builder: (context, box) => SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          child: ConstrainedBox(
            constraints: BoxConstraints(minWidth: box.maxWidth),
            child: Text(
              text,
              key: textKey,
              style: ts(context.type.code, color ?? c.textPrimary, mono: true),
            ),
          ),
        ),
      ),
    );
  }
}

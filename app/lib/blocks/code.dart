// Code: monospace text in a grey box; Enter inserts a line break. Long
// lines do not wrap: the box scrolls sideways.

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../editor/model.dart';
import '../registry/block_codecs.dart';
import '../registry/block_registry.dart';
import '../ui/ui.dart';

TextStyle _style(Node n, AppColors c, TypeScale ty) => ts(ty.code, c.textPrimary, mono: true);

/// The label above the box: the block's target (its fence info string, such as
/// `dart`), or "Plain text" when there is none.
String _label(Node n) {
  final info = n.target?.trim() ?? '';
  return info.isEmpty ? 'Plain text' : info;
}

Widget _decorate(BlockContext b, Widget text) => Container(
      decoration: BoxDecoration(color: b.colors.bgSecondary, borderRadius: BorderRadius.circular(Radii.sm)),
      padding: const EdgeInsets.fromLTRB(16, 32, 16, 16),
      child: Stack(clipBehavior: Clip.none, children: [
        Positioned(
          left: 0,
          right: 0,
          top: -24,
          child: Text(
            _label(b.node),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: ts(TypeScale.desktop.uiSmall, b.colors.textSecondary, weight: 400),
          ),
        ),
        LayoutBuilder(
          builder: (context, box) => SingleChildScrollView(
            key: ValueKey('code-scroll-${b.node.id}'),
            scrollDirection: Axis.horizontal,
            child: ConstrainedBox(
              constraints: BoxConstraints(minWidth: box.maxWidth),
              child: IntrinsicWidth(child: text),
            ),
          ),
        ),
      ]),
    );

double _handleTop(BlockContext b) => 2;

/// The fence's info string: the target, unless it could break the fence.
String _fenceInfo(Node n) {
  final info = n.target?.trim() ?? '';
  return info.isEmpty || RegExp(r'[`\r\n]').hasMatch(info) ? '' : info;
}

List<String> _fenceLines(Node n, String inline) => ['```${_fenceInfo(n)}', ...n.text.split('\n'), '```'];

String _plainLine(Node n) => n.text;

// The code's own text, unmarked and unindented; the writer indents each line.
const _codeCodecs = BlockCodecs(
  markdown: MarkdownCodec(_fenceLines),
  clipboard: ClipboardCodec.viaMarkdown(),
  plainText: PlainTextCodec(_plainLine),
);

const codeBlock = BlockType(
  kind: Kinds.code,
  label: 'Code',
  hint: 'Capture a code snippet.',
  icon: Ico.code,
  aliases: 'code snippet',
  shortcut: '```',
  markdown: ['```'],
  multiline: true,
  textStyle: _style,
  decorate: _decorate,
  handleTop: _handleTop,
  codecs: _codeCodecs,
);

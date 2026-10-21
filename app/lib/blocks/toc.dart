// Table of contents: the page's headings as links, indented by level
// (14/18.2 textSecondary with a thin underline, 30px apart).
// Tapping one scrolls to the heading.

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../editor/model.dart';
import '../registry/block_registry.dart';
import '../ui/icon_x.dart';
import '../ui/ui.dart';

/// Heading level of [n] (1..3), or 0 for other kinds.
int headingLevel(Node n) => switch (n.kind) {
      Kinds.heading1 || Kinds.toggleHeading1 => 1,
      Kinds.heading2 || Kinds.toggleHeading2 => 2,
      Kinds.heading3 || Kinds.toggleHeading3 => 3,
      _ => 0,
    };

Widget _body(BlockContext b) {
  final c = b.colors;
  final heads = [for (final n in b.host.readingOrder) if (headingLevel(n) > 0) n];
  final style = tsRaw(14, 18.2, 400, c.textSecondary);
  if (heads.isEmpty) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Text('Add headings to create a table of contents.', style: style.copyWith(color: c.textTertiary)),
    );
  }
  final top = heads.map(headingLevel).reduce((a, x) => a < x ? a : x);
  return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
    for (final h in heads)
      Hover(
        key: ValueKey('toc-${h.id}'),
        radius: Radii.sm,
        onTap: () => b.host.revealBlock(h),
        child: Container(
          height: 30,
          alignment: Alignment.centerLeft,
          padding: EdgeInsets.only(left: 2 + 24.0 * (headingLevel(h) - top), right: 2),
          child: Container(
            decoration: BoxDecoration(border: Border(bottom: BorderSide(color: c.divider))),
            child: Text(h.text.isEmpty ? 'Untitled' : h.text,
                maxLines: 1, overflow: TextOverflow.ellipsis, style: style),
          ),
        ),
      ),
  ]);
}

const tocBlock = BlockType(
  kind: Kinds.toc,
  label: 'Table of contents',
  hint: 'Show an outline of your page.',
  iconX: Ix.toc,
  aliases: 'toc table contents outline',
  editable: false,
  acceptsChildren: false,
  group: 'Advanced',
  body: _body,
);

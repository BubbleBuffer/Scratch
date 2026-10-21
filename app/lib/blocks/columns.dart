// Columns: a column list holds column blocks side by side (46px apart); each column's blocks are ordinary rows. On mobile the
// columns stack. The `/` menu offers "2 columns" and "3 columns", which the
// editor builds (see PageEditorState.insertColumns).

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../registry/block_registry.dart';
import '../ui/icon_x.dart';

/// Space between columns.
const columnGap = 46.0;

/// Room left of each column's blocks for their drag handles (desktop).
const columnGutter = 48.0;

Widget _body(BlockContext b) {
  final cols = [for (final c in b.node.children) if (c.kind == Kinds.column) c];
  if (cols.isEmpty) return const SizedBox(height: 24);
  return LayoutBuilder(builder: (context, box) {
    final w = box.maxWidth;
    if (b.mobile) {
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        for (final c in cols) b.host.nestedBlocks(c, w, 0),
      ]);
    }
    final k = cols.length;
    final text = w - columnGutter;
    final cw = (text - columnGap * (k - 1)) / k;
    return Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
      for (var i = 0; i < k; i++)
        SizedBox(
          width: cw + (i == 0 ? columnGutter : columnGap),
          child: b.host.nestedBlocks(cols[i], cw, i == 0 ? columnGutter : columnGap),
        ),
    ]);
  });
}

const columnListBlock = BlockType(
  kind: Kinds.columnList,
  label: 'Columns',
  iconX: Ix.columns,
  editable: false,
  inMenu: false,
  colorable: false,
  children: ChildMode.embedded,
  body: _body,
);

const columnBlock = BlockType(
  kind: Kinds.column,
  label: 'Column',
  iconX: Ix.columns,
  editable: false,
  inMenu: false,
  colorable: false,
);

/// `/` menu entries; not kinds of their own.
const columns2Entry = BlockType(
  kind: 'columns2',
  label: '2 columns',
  hint: 'Create 2 columns of blocks.',
  iconX: Ix.columns,
  aliases: 'columns two',
  editable: false,
  insertOnly: true,
  group: 'Advanced',
);

const columns3Entry = BlockType(
  kind: 'columns3',
  label: '3 columns',
  hint: 'Create 3 columns of blocks.',
  iconX: Ix.columns,
  aliases: 'columns three',
  editable: false,
  insertOnly: true,
  group: 'Advanced',
);

// To-do: a checkbox before the text; checked items are struck through.

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../editor/model.dart';
import '../registry/block_registry.dart';
import '../ui/ui.dart';
import 'heading.dart';

TextStyle _style(Node n, AppColors c, TypeScale ty) {
  final s = ts(ty.body, c.textPrimary);
  if (!n.checked) return s;
  return s.copyWith(
    color: c.textTertiary,
    decoration: TextDecoration.lineThrough,
    decorationColor: c.textTertiary,
  );
}

Widget _decorate(BlockContext b, Widget text) {
  final n = b.node;
  final host = b.host;
  final patch = spacePatch(
    host is HeadingProgressHost
        ? (host as HeadingProgressHost).headingProgressSpaceColor
        : null,
    b.colors.dark ? Brightness.dark : Brightness.light,
  );
  final box = StacksCheckbox(
    key: ValueKey('check-${n.id}'),
    checked: n.checked,
    fill: patch?.fill,
    ink: patch?.ink,
    size: todoBoxSize,
    onChanged: (_) => b.host.toggleChecked(n),
  );
  // The box sits in the 24 px marker column, centred on the first line, with
  // [todoTextGap] between it and the text.
  const inset = (24 - todoBoxSize) / 2;
  return Row(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      SizedBox(
        width: 24,
        height: b.lineHeight,
        child: Center(child: box),
      ),
      const SizedBox(width: todoTextGap - inset),
      Expanded(child: text),
    ],
  );
}

/// The to-do box's size and the gap between it and the text.
const todoBoxSize = 22.0, todoTextGap = 8.0;

const todoBlock = BlockType(
  kind: Kinds.todo,
  label: 'To-do list',
  hint: 'Track tasks with a to-do list.',
  icon: Ico.todo,
  aliases: 'todo to-do checkbox task',
  shortcut: '[]',
  markdown: ['[] ', '[ ] '],
  listItem: true,
  exitOnEmptyEnter: true,
  placeholder: 'To-do',
  textStyle: _style,
  decorate: _decorate,
);

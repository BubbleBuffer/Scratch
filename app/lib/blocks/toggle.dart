// Toggle and toggle headings: a triangle button before the text that shows
// or hides the block's children. Open state is per device (the editor
// keeps it); an open toggle without children offers "Empty toggle".

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../editor/model.dart';
import '../registry/block_registry.dart';
import '../ui/icon_x.dart';
import '../ui/ui.dart';
import 'heading.dart';

TextStyle _h1(Node n, AppColors c, TypeScale ty) =>
    tsDisplay(ty.h1, c.textPrimary);
TextStyle _h2(Node n, AppColors c, TypeScale ty) =>
    tsDisplay(ty.h2, c.textPrimary);
TextStyle _h3(Node n, AppColors c, TypeScale ty) =>
    tsDisplay(ty.h3, c.textPrimary);
double _m1(bool mobile) => mobile ? 20 : 32;
double _m2(bool mobile) => mobile ? 16 : 22;
double _m3(bool mobile) => mobile ? 12 : 16;

/// The toggle button (24x24, a triangle that turns when open).
class ToggleButton extends StatelessWidget {
  const ToggleButton({
    super.key,
    required this.open,
    required this.onTap,
    this.size = 24,
  });
  final bool open;
  final VoidCallback onTap;
  final double size;

  @override
  Widget build(BuildContext context) => Hover(
    onTap: onTap,
    radius: Radii.sm,
    child: SizedBox(
      width: size,
      height: size,
      child: Center(
        child: AnimatedRotation(
          turns: open ? 0.25 : 0,
          duration: Motion.of(context).fast,
          child: IconX(
            Ix.triangle,
            size: 16,
            color: context.colors.iconPrimary,
          ),
        ),
      ),
    ),
  );
}

Widget _decorate(BlockContext b, Widget text) {
  final n = b.node;
  final open = b.host.isOpen(n);
  final row = Row(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      Padding(
        padding: EdgeInsets.only(top: math.max(0, (b.lineHeight - 24) / 2)),
        child: ToggleButton(
          key: ValueKey('toggle-${n.id}'),
          open: open,
          onTap: () => b.host.toggleOpen(n),
        ),
      ),
      const SizedBox(width: 4),
      Expanded(
        child: headingLevel(n.kind) == null
            ? text
            : decorateHeadingProgress(b, text),
      ),
    ],
  );
  if (!open || n.children.isNotEmpty) return row;
  return Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      row,
      GestureDetector(
        key: ValueKey('toggle-empty-${n.id}'),
        behavior: HitTestBehavior.opaque,
        onTap: () => b.host.addChild(n),
        child: Padding(
          padding: const EdgeInsets.only(left: 28, top: 4),
          child: Text(
            'Empty toggle. Click or drop blocks inside.',
            style: ts(
              b.mobile ? b.type.body : b.type.ui,
              b.colors.textTertiary,
              weight: 400,
            ),
          ),
        ),
      ),
    ],
  );
}

double _handleTop(BlockContext b) =>
    (b.mobile ? 3 : 2) + (b.lineHeight - 24) / 2;

const toggleBlock = BlockType(
  kind: Kinds.toggle,
  label: 'Toggle list',
  hint: 'Toggles can hide and show content inside.',
  iconX: Ix.triangle,
  aliases: 'toggle collapse dropdown',
  shortcut: '>',
  markdown: ['> '],
  placeholder: 'Toggle',
  children: ChildMode.toggle,
  decorate: _decorate,
  handleTop: _handleTop,
);

const toggleHeading1Block = BlockType(
  kind: Kinds.toggleHeading1,
  label: 'Toggle heading 1',
  hint: 'Hide content inside a large heading.',
  iconText: '▸H1',
  aliases: 'toggleheading1 toggle heading h1',
  shortcut: '#>',
  markdown: ['#> '],
  placeholder: 'Toggle heading 1',
  alwaysShowPlaceholder: true,
  children: ChildMode.toggle,
  textStyle: _h1,
  topMargin: _m1,
  decorate: _decorate,
);

const toggleHeading2Block = BlockType(
  kind: Kinds.toggleHeading2,
  label: 'Toggle heading 2',
  hint: 'Hide content inside a medium heading.',
  iconText: '▸H2',
  aliases: 'toggleheading2 toggle heading h2',
  shortcut: '##>',
  markdown: ['##> '],
  placeholder: 'Toggle heading 2',
  alwaysShowPlaceholder: true,
  children: ChildMode.toggle,
  textStyle: _h2,
  topMargin: _m2,
  decorate: _decorate,
);

const toggleHeading3Block = BlockType(
  kind: Kinds.toggleHeading3,
  label: 'Toggle heading 3',
  hint: 'Hide content inside a small heading.',
  iconText: '▸H3',
  aliases: 'toggleheading3 toggle heading h3',
  shortcut: '###>',
  markdown: ['###> '],
  placeholder: 'Toggle heading 3',
  alwaysShowPlaceholder: true,
  children: ChildMode.toggle,
  textStyle: _h3,
  topMargin: _m3,
  decorate: _decorate,
);

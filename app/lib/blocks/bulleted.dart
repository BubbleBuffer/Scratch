// Bulleted list: disc, circle, square by depth.

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../registry/block_registry.dart';
import '../ui/ui.dart';
import 'list_marker.dart';

Widget _decorate(BlockContext b, Widget text) => markerRow(
      b,
      Center(
          child: CustomPaint(
              size: const Size.square(ListMarker.bullet), painter: _BulletPainter(b.node.depth % 3, b.colors.accent))),
      text,
    );

const bulletedBlock = BlockType(
  kind: Kinds.bulleted,
  label: 'Bulleted list',
  hint: 'Create a simple bulleted list.',
  icon: Ico.bullets,
  aliases: 'bulleted list bullet ul',
  shortcut: '-',
  markdown: ['- ', '* '],
  listItem: true,
  exitOnEmptyEnter: true,
  placeholder: 'List',
  decorate: _decorate,
);

class _BulletPainter extends CustomPainter {
  _BulletPainter(this.level, this.color);
  final int level;
  final Color color;
  @override
  void paint(Canvas canvas, Size size) {
    final ctr = size.center(Offset.zero);
    switch (level) {
      case 0:
        canvas.drawCircle(ctr, ListMarker.disc, Paint()..color = color);
      case 1:
        canvas.drawCircle(
            ctr,
            ListMarker.ring,
            Paint()
              ..color = color
              ..style = PaintingStyle.stroke
              ..strokeWidth = ListMarker.ringStroke);
      default:
        canvas.drawRect(Rect.fromCenter(center: ctr, width: ListMarker.square, height: ListMarker.square), Paint()..color = color);
    }
  }

  @override
  bool shouldRepaint(_BulletPainter old) => old.level != level || old.color != color;
}

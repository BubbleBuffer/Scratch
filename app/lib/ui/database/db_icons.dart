// Database icons: property types, view layouts and toolbar glyphs, drawn
// like the UI kit's icons (thin strokes on a 20-unit grid).

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

import '../ui.dart';

enum DbIco {
  title,
  text,
  number,
  select,
  multiSelect,
  status,
  date,
  checkbox,
  url,
  email,
  phone,
  relation,
  createdTime,
  editedTime,
  table,
  board,
  list,
  gallery,
  calendar,
  timeline,
  filter,
  sort,
  lightning,
  sliders,
  peekSide,
  peekCenter,
  fullPage,
  eye,
  eyeOff,
  edit,
  wrap,
  pin,
  insertLeft,
  insertRight,
  group,
  sum,
  template,
  layout,
  database,
  files,
  formula,
  rollup,
}

class DbIcon extends StatelessWidget {
  const DbIcon(this.icon, {super.key, this.size = 20, required this.color, this.stroke = 1.5});
  final DbIco icon;
  final double size;
  final Color color;
  final double stroke;

  @override
  Widget build(BuildContext context) => SizedBox(
        width: size,
        height: size,
        child: CustomPaint(painter: _DbIconPainter(icon, color, stroke)),
      );
}

class _DbIconPainter extends CustomPainter {
  _DbIconPainter(this.icon, this.color, this.stroke);
  final DbIco icon;
  final Color color;
  final double stroke;

  @override
  void paint(Canvas canvas, Size size) {
    final k = size.width / 20;
    canvas.scale(k);
    final p = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round
      ..strokeWidth = stroke / k;
    final fill = Paint()
      ..color = color
      ..style = PaintingStyle.fill;
    void line(double x1, double y1, double x2, double y2) => canvas.drawLine(Offset(x1, y1), Offset(x2, y2), p);
    void poly(List<double> xy, {bool close = false, Paint? paint}) {
      final path = Path()..moveTo(xy[0], xy[1]);
      for (var i = 2; i < xy.length; i += 2) {
        path.lineTo(xy[i], xy[i + 1]);
      }
      if (close) path.close();
      canvas.drawPath(path, paint ?? p);
    }

    void rrect(double l, double t, double r, double b, double rad, {Paint? paint}) =>
        canvas.drawRRect(RRect.fromLTRBR(l, t, r, b, Radius.circular(rad)), paint ?? p);
    void dot(double x, double y, double r) => canvas.drawCircle(Offset(x, y), r, fill);
    void circle(double x, double y, double r) => canvas.drawCircle(Offset(x, y), r, p);
    void text(String s, double fontSize, {int weight = 500}) {
      final tp = TextPainter(
        textDirection: TextDirection.ltr,
        text: TextSpan(text: s, style: TextStyle(fontFamily: sansFamily, fontSize: fontSize, color: color, fontWeight: fw(weight))),
      )..layout();
      tp.paint(canvas, Offset(10 - tp.width / 2, 10 - tp.height / 2));
    }

    void clock() {
      circle(10, 10, 6.5);
      poly([10, 6.5, 10, 10, 12.8, 11.6]);
    }

    void calendarBox() {
      rrect(3.5, 4.5, 16.5, 16.5, 2);
      line(3.5, 8.3, 16.5, 8.3);
      line(7, 3, 7, 5.5);
      line(13, 3, 13, 5.5);
    }

    void eye() {
      final path = Path()
        ..moveTo(2.5, 10)
        ..quadraticBezierTo(10, 1.5, 17.5, 10)
        ..quadraticBezierTo(10, 18.5, 2.5, 10)
        ..close();
      canvas.drawPath(path, p);
      circle(10, 10, 2.5);
    }

    switch (icon) {
      case DbIco.title:
        text('Aa', 11.5, weight: 600);
      case DbIco.text:
        line(4, 5.5, 16, 5.5);
        line(4, 10, 16, 10);
        line(4, 14.5, 11, 14.5);
      case DbIco.number:
        line(8.3, 4, 6.8, 16);
        line(13.3, 4, 11.8, 16);
        line(4.2, 7.8, 16.2, 7.8);
        line(3.8, 12.2, 15.8, 12.2);
      case DbIco.select:
        circle(10, 10, 6.5);
        poly([7, 8.6, 10, 11.8, 13, 8.6], close: true, paint: fill);
      case DbIco.multiSelect:
        for (final y in [5.5, 10.0, 14.5]) {
          dot(4.5, y, 1.25);
          line(8, y, 16.5, y);
        }
      case DbIco.status:
        for (var i = 0; i < 8; i++) {
          final a = i * math.pi / 4 - math.pi / 2;
          canvas.drawArc(Rect.fromCircle(center: const Offset(10, 10), radius: 6.5), a + 0.12, math.pi / 4 - 0.4, false, p);
        }
        dot(10, 10, 2.2);
      case DbIco.date || DbIco.calendar:
        calendarBox();
        if (icon == DbIco.calendar) {
          for (final y in [11.0, 13.8]) {
            for (final x in [7.0, 10.0, 13.0]) {
              dot(x, y, 0.9);
            }
          }
        }
      case DbIco.checkbox:
        rrect(3.5, 3.5, 16.5, 16.5, 3);
        poly([6.8, 10.2, 9, 12.5, 13.4, 7.6]);
      case DbIco.url:
        line(8.5, 11.5, 11.5, 8.5);
        canvas.drawPath(
            Path()
              ..moveTo(9.5, 6)
              ..lineTo(11, 4.5)
              ..arcToPoint(const Offset(15.5, 9), radius: const Radius.circular(3.2))
              ..lineTo(14, 10.5),
            p);
        canvas.drawPath(
            Path()
              ..moveTo(10.5, 14)
              ..lineTo(9, 15.5)
              ..arcToPoint(const Offset(4.5, 11), radius: const Radius.circular(3.2))
              ..lineTo(6, 9.5),
            p);
      case DbIco.email:
        circle(10, 10, 2.8);
        canvas.drawPath(
            Path()
              ..moveTo(12.8, 7.3)
              ..lineTo(12.8, 11.2)
              ..quadraticBezierTo(12.8, 13, 14.6, 13)
              ..quadraticBezierTo(16.6, 13, 16.6, 10)
              ..arcToPoint(const Offset(13.6, 15.6), radius: const Radius.circular(6.6), largeArc: true, clockwise: false),
            p);
      case DbIco.phone:
        canvas.drawPath(
            Path()
              ..moveTo(6.2, 3.6)
              ..lineTo(8.3, 3.6)
              ..lineTo(9.4, 7)
              ..lineTo(7.9, 8.4)
              ..quadraticBezierTo(9, 11.2, 11.6, 12.2)
              ..lineTo(13, 10.7)
              ..lineTo(16.4, 11.8)
              ..lineTo(16.4, 13.9)
              ..quadraticBezierTo(16.4, 16.4, 13.9, 16.4)
              ..quadraticBezierTo(4.2, 15.8, 3.6, 6.1)
              ..quadraticBezierTo(3.6, 3.6, 6.2, 3.6)
              ..close(),
            p);
      case DbIco.relation:
        line(5, 15, 15, 5);
        poly([7.5, 5, 15, 5, 15, 12.5]);
      case DbIco.createdTime:
        clock();
      case DbIco.editedTime:
        clock();
        dot(15.5, 15.5, 2);
      case DbIco.table:
        rrect(3, 4.5, 17, 15.5, 2);
        line(3, 8.3, 17, 8.3);
        line(3, 11.9, 17, 11.9);
        line(8, 4.5, 8, 15.5);
      case DbIco.board:
        rrect(3, 4.5, 17, 15.5, 2);
        line(7.7, 4.5, 7.7, 15.5);
        line(12.3, 4.5, 12.3, 15.5);
      case DbIco.list:
        for (final y in [5.5, 10.0, 14.5]) {
          dot(4.5, y, 1.25);
          line(8, y, 16.5, y);
        }
      case DbIco.gallery:
        rrect(3.5, 4, 9, 9.5, 1.5);
        rrect(11, 4, 16.5, 9.5, 1.5);
        rrect(3.5, 10.5, 9, 16, 1.5);
        rrect(11, 10.5, 16.5, 16, 1.5);
      case DbIco.timeline:
        line(3.5, 5.5, 10.5, 5.5);
        line(7, 10, 16.5, 10);
        line(5, 14.5, 12, 14.5);
      case DbIco.filter:
        line(3.5, 6, 16.5, 6);
        line(6, 10, 14, 10);
        line(8.5, 14, 11.5, 14);
      case DbIco.sort:
        line(7, 4, 7, 16);
        poly([4.2, 6.8, 7, 4, 9.8, 6.8]);
        line(13, 4, 13, 16);
        poly([10.2, 13.2, 13, 16, 15.8, 13.2]);
      case DbIco.lightning:
        poly([11.2, 2.8, 5, 11, 9.8, 11, 8.8, 17.2, 15, 9, 10.2, 9, 11.2, 2.8], close: true);
      case DbIco.sliders:
        line(3.5, 6.5, 16.5, 6.5);
        line(3.5, 13.5, 16.5, 13.5);
        canvas.drawCircle(const Offset(12.5, 6.5), 2, Paint()..color = const Color(0x00000000));
        circle(12.5, 6.5, 2);
        circle(7.5, 13.5, 2);
      case DbIco.peekSide:
        rrect(3, 4, 17, 16, 2);
        rrect(10.5, 5.5, 15.5, 14.5, 1, paint: fill);
      case DbIco.peekCenter:
        rrect(3, 4, 17, 16, 2);
        rrect(6.5, 6.5, 13.5, 13.5, 1, paint: fill);
      case DbIco.fullPage:
        rrect(3, 4, 17, 16, 2);
        rrect(5, 6, 15, 14, 1, paint: fill);
      case DbIco.eye:
        eye();
      case DbIco.eyeOff:
        eye();
        line(4, 4, 16, 16);
      case DbIco.edit:
        poly([4, 16, 4.6, 12.8, 13.4, 4, 16, 6.6, 7.2, 15.4], close: true);
        line(11.4, 6, 14, 8.6);
      case DbIco.wrap:
        line(3.5, 5, 16.5, 5);
        canvas.drawPath(
            Path()
              ..moveTo(3.5, 10)
              ..lineTo(13.5, 10)
              ..arcToPoint(const Offset(13.5, 15), radius: const Radius.circular(2.5))
              ..lineTo(10, 15),
            p);
        poly([11.8, 13.2, 10, 15, 11.8, 16.8]);
        line(3.5, 15, 7, 15);
      case DbIco.pin:
        poly([7.5, 3.5, 12.5, 3.5, 11.8, 8, 14.5, 11, 5.5, 11, 8.2, 8], close: true);
        line(10, 11, 10, 16.5);
      case DbIco.insertLeft:
        line(15.5, 4, 15.5, 16);
        line(12, 10, 4, 10);
        poly([7, 7, 4, 10, 7, 13]);
      case DbIco.insertRight:
        line(4.5, 4, 4.5, 16);
        line(8, 10, 16, 10);
        poly([13, 7, 16, 10, 13, 13]);
      case DbIco.group:
        rrect(3.5, 4, 16.5, 8.5, 1.5);
        rrect(3.5, 11.5, 16.5, 16, 1.5);
      case DbIco.sum:
        poly([15, 5, 5, 5, 10.5, 10, 5, 15, 15, 15]);
      case DbIco.template:
        rrect(4.5, 3, 15.5, 17, 2);
        line(7.5, 7, 12.5, 7);
        line(7.5, 10, 12.5, 10);
        line(7.5, 13, 10.5, 13);
      case DbIco.layout:
        rrect(3, 4, 17, 16, 2);
        line(3, 8, 17, 8);
        line(8, 8, 8, 16);
      case DbIco.database:
        rrect(3, 4.5, 17, 15.5, 2);
        line(3, 8.3, 17, 8.3);
        line(8, 8.3, 8, 15.5);
        line(3, 11.9, 17, 11.9);
      case DbIco.files:
        // A paperclip.
        canvas.drawPath(
            Path()
              ..moveTo(13.5, 7.5)
              ..lineTo(8.2, 12.8)
              ..arcToPoint(const Offset(9.9, 14.5), radius: const Radius.circular(1.2), clockwise: false)
              ..lineTo(15.4, 9)
              ..arcToPoint(const Offset(11.2, 4.8), radius: const Radius.circular(3), clockwise: false)
              ..lineTo(5.6, 10.4)
              ..arcToPoint(const Offset(11.5, 16.3), radius: const Radius.circular(4.2), clockwise: false)
              ..lineTo(16, 11.8),
            p);
      case DbIco.formula:
        // A sigma.
        poly([15, 4.5, 5, 4.5, 10.5, 10, 5, 15.5, 15, 15.5]);
      case DbIco.rollup:
        // A magnifying glass.
        circle(8.8, 8.8, 4.6);
        line(12.2, 12.2, 16, 16);
    }
  }

  @override
  bool shouldRepaint(_DbIconPainter old) => old.icon != icon || old.color != color || old.stroke != stroke;
}

/// The layout icon of a view kind.
DbIco viewIcon(ViewKindName kind) => switch (kind) {
      ViewKindName.table => DbIco.table,
      ViewKindName.board => DbIco.board,
      ViewKindName.list => DbIco.list,
      ViewKindName.gallery => DbIco.gallery,
      ViewKindName.calendar => DbIco.calendar,
      ViewKindName.timeline => DbIco.timeline,
    };

/// Mirrors the bridge's ViewKind so this file does not import the bridge.
enum ViewKindName { table, board, list, gallery, calendar, timeline }

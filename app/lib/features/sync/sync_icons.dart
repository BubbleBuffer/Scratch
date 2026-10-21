// Line icons for sync and settings, drawn like the UI kit's (20-unit grid,
// 1.5 px stroke), kept here so the kit's icon set stays unchanged.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

enum SyncIco { cloud, cloudCheck, cloudSync, cloudOff, cloudAlert, device, devices, conflict, folder, info, sun, qr }

class SyncIcon extends StatelessWidget {
  const SyncIcon(this.icon, {super.key, this.size = 20, required this.color});
  final SyncIco icon;
  final double size;
  final Color color;

  @override
  Widget build(BuildContext context) =>
      SizedBox(width: size, height: size, child: CustomPaint(painter: _Painter(icon, color)));
}

class _Painter extends CustomPainter {
  _Painter(this.icon, this.color);
  final SyncIco icon;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final k = size.width / 20;
    canvas.scale(k);
    final p = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round
      ..strokeWidth = 1.5 / k;
    final fill = Paint()..color = color;
    void line(double x1, double y1, double x2, double y2) => canvas.drawLine(Offset(x1, y1), Offset(x2, y2), p);
    void rrect(double l, double t, double r, double b, double rad) =>
        canvas.drawRRect(RRect.fromLTRBR(l, t, r, b, Radius.circular(rad)), p);

    Path cloud() => Path()
      ..moveTo(6, 15.5)
      ..arcToPoint(const Offset(5.6, 8.6), radius: const Radius.circular(3.5), largeArc: true)
      ..arcToPoint(const Offset(14.2, 7.6), radius: const Radius.circular(4.6))
      ..arcToPoint(const Offset(14.5, 15.5), radius: const Radius.circular(4), largeArc: true)
      ..close();

    switch (icon) {
      case SyncIco.cloud:
        canvas.drawPath(cloud(), p);
      case SyncIco.cloudCheck:
        canvas.drawPath(cloud(), p);
        final c = Path()
          ..moveTo(7.6, 11.6)
          ..lineTo(9.4, 13.2)
          ..lineTo(12.6, 10);
        canvas.drawPath(c, p);
      case SyncIco.cloudSync:
        canvas.drawPath(cloud(), p);
        canvas.drawArc(Rect.fromCircle(center: const Offset(10.2, 11.8), radius: 2.2), -math.pi * 0.9, math.pi * 1.4, false, p);
        line(12.4, 9.2, 12.4, 10.6);
        line(12.4, 10.6, 11.0, 10.6);
      case SyncIco.cloudOff:
        canvas.drawPath(cloud(), p);
        line(3, 3.5, 17, 17.5);
      case SyncIco.cloudAlert:
        canvas.drawPath(cloud(), p);
        line(10.2, 9.6, 10.2, 11.8);
        canvas.drawCircle(const Offset(10.2, 13.6), 0.9, fill);
      case SyncIco.device:
        rrect(6, 2.5, 14, 17.5, 2);
        line(9, 15, 11, 15);
      case SyncIco.devices:
        rrect(2.5, 4, 13.5, 12, 1.5);
        line(5.5, 15, 10.5, 15);
        line(8, 12, 8, 15);
        rrect(12.5, 7.5, 17.5, 16, 1.2);
      case SyncIco.conflict:
        // Two arrows in opposite directions.
        line(3.5, 7, 15.5, 7);
        line(12.5, 4, 15.5, 7);
        line(12.5, 10, 15.5, 7);
        line(16.5, 13, 4.5, 13);
        line(7.5, 10, 4.5, 13);
        line(7.5, 16, 4.5, 13);
      case SyncIco.folder:
        final f = Path()
          ..moveTo(2.5, 5.5)
          ..quadraticBezierTo(2.5, 4, 4, 4)
          ..lineTo(7.5, 4)
          ..lineTo(9.3, 6)
          ..lineTo(16, 6)
          ..quadraticBezierTo(17.5, 6, 17.5, 7.5)
          ..lineTo(17.5, 14.5)
          ..quadraticBezierTo(17.5, 16, 16, 16)
          ..lineTo(4, 16)
          ..quadraticBezierTo(2.5, 16, 2.5, 14.5)
          ..close();
        canvas.drawPath(f, p);
      case SyncIco.info:
        canvas.drawCircle(const Offset(10, 10), 7, p);
        line(10, 9, 10, 13.5);
        canvas.drawCircle(const Offset(10, 6.6), 0.9, fill);
      case SyncIco.sun:
        canvas.drawCircle(const Offset(10, 10), 3.2, p);
        for (var i = 0; i < 8; i++) {
          final a = i * math.pi / 4;
          line(10 + 5.4 * math.cos(a), 10 + 5.4 * math.sin(a), 10 + 7 * math.cos(a), 10 + 7 * math.sin(a));
        }
      case SyncIco.qr:
        rrect(3, 3, 8.5, 8.5, 1);
        rrect(11.5, 3, 17, 8.5, 1);
        rrect(3, 11.5, 8.5, 17, 1);
        canvas.drawRect(const Rect.fromLTRB(12, 12, 14, 14), fill);
        canvas.drawRect(const Rect.fromLTRB(15, 15, 17, 17), fill);
    }
  }

  @override
  bool shouldRepaint(_Painter old) => old.icon != icon || old.color != color;
}

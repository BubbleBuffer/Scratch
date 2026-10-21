import 'package:flutter/widgets.dart';

import 'tokens.dart';

/// The approved prototype mark; deliberately independent of theme and space.
class CalicoMark extends StatelessWidget {
  const CalicoMark({super.key, this.size = 28});
  final double size;
  @override
  Widget build(BuildContext context) => ExcludeSemantics(
    child: CustomPaint(
      size: Size.square(size),
      painter: const CalicoMarkPainter(),
    ),
  );
}

class CalicoMarkPainter extends CustomPainter {
  const CalicoMarkPainter();
  @override
  void paint(Canvas canvas, Size size) {
    canvas.scale(size.width / 32, size.height / 32);
    final outline = Path()
      ..moveTo(7, 1.5)
      ..lineTo(24.5, 1.5)
      ..cubicTo(28, 1.5, 30.5, 4, 30.5, 7.5)
      ..lineTo(30.5, 24)
      ..cubicTo(30.5, 28, 28, 30.5, 24, 30.5)
      ..lineTo(6.5, 30.5)
      ..cubicTo(3.5, 30.5, 1.5, 28.5, 1.5, 25.5)
      ..lineTo(1.5, 7)
      ..cubicTo(1.5, 3.8, 3.8, 1.5, 7, 1.5)
      ..close();
    canvas.drawPath(outline, Paint()..color = const Color(0xFFFFFAF1));
    canvas.drawPath(
      outline,
      Paint()
        ..color = const Color(0xFF282A2F)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.5,
    );
    canvas.drawPath(
      Path()
        ..moveTo(1.5, 7)
        ..cubicTo(1.5, 3.8, 3.8, 1.5, 7, 1.5)
        ..lineTo(17, 1.5)
        ..lineTo(14, 13)
        ..lineTo(1.5, 16)
        ..close(),
      Paint()..color = const Color(0xFFDE945F),
    );
    canvas.drawPath(
      Path()
        ..moveTo(30.5, 18)
        ..lineTo(30.5, 24)
        ..cubicTo(30.5, 28, 28, 30.5, 24, 30.5)
        ..lineTo(17, 30.5)
        ..lineTo(19, 21)
        ..close(),
      Paint()..color = const Color(0xFF282A2F),
    );
    canvas.drawPath(
      Path()
        ..moveTo(20, 1.5)
        ..lineTo(24.5, 1.5)
        ..cubicTo(28, 1.5, 30.5, 4, 30.5, 7.5)
        ..lineTo(30.5, 10)
        ..lineTo(23, 8)
        ..close(),
      Paint()..color = const Color(0xFFDDA39D),
    );
  }

  @override
  bool shouldRepaint(CalicoMarkPainter oldDelegate) => false;
}

/// Exact 200 x 140 cat-loaf paths from the approved calico prototype.
class SleepingCatLoaf extends StatelessWidget {
  const SleepingCatLoaf({
    super.key,
    required this.patch,
    this.width = 200,
    this.height = 140,
  });
  final SpacePatch patch;
  final double width, height;
  @override
  Widget build(BuildContext context) => Semantics(
    label: 'A calico cat loaf, asleep',
    image: true,
    child: CustomPaint(
      size: Size(width, height),
      painter: SleepingCatPainter(
        patch: patch,
        sheet: context.colors.bgPrimary,
        outline: context.colors.textPrimary,
        ground: context.colors.border,
      ),
    ),
  );
}

class SleepingCatPainter extends CustomPainter {
  const SleepingCatPainter({
    required this.patch,
    required this.sheet,
    required this.outline,
    required this.ground,
  });
  final SpacePatch patch;
  final Color sheet, outline, ground;
  @override
  void paint(Canvas canvas, Size size) {
    canvas.scale(size.width / 200, size.height / 140);
    void fill(Path path, Color color) =>
        canvas.drawPath(path, Paint()..color = color);
    void stroke(Path path, double width) => canvas.drawPath(
      path,
      Paint()
        ..color = outline
        ..style = PaintingStyle.stroke
        ..strokeWidth = width
        ..strokeCap = StrokeCap.round
        ..strokeJoin = StrokeJoin.round,
    );
    final loaf = Path()
      ..moveTo(30, 122)
      ..cubicTo(30, 74, 58, 50, 104, 50)
      ..cubicTo(148, 50, 172, 76, 172, 122)
      ..close();
    final head = Path()
      ..addOval(Rect.fromCircle(center: Offset(70, 86), radius: 30));
    canvas.drawRRect(
      RRect.fromRectAndRadius(
        const Rect.fromLTWH(16, 121, 172, 3),
        const Radius.circular(1.5),
      ),
      Paint()..color = ground,
    );
    fill(loaf, sheet);
    stroke(loaf, 3);
    canvas.save();
    canvas.clipPath(loaf);
    fill(
      Path()
        ..moveTo(112, 46)
        ..cubicTo(146, 48, 172, 76, 176, 112)
        ..lineTo(136, 116)
        ..cubicTo(130, 92, 120, 72, 112, 46)
        ..close(),
      patch.fill,
    );
    fill(
      Path()
        ..moveTo(150, 96)
        ..cubicTo(160, 100, 170, 110, 176, 124)
        ..lineTo(144, 124)
        ..close(),
      patch.deep,
    );
    canvas.restore();
    final leftEar = Path()
      ..moveTo(48, 66)
      ..lineTo(50, 42)
      ..lineTo(66, 56)
      ..close();
    fill(leftEar, patch.deep);
    stroke(leftEar, 3);
    final rightEar = Path()
      ..moveTo(76, 56)
      ..lineTo(90, 42)
      ..lineTo(94, 64)
      ..close();
    fill(rightEar, sheet);
    stroke(rightEar, 3);
    fill(head, sheet);
    stroke(head, 3);
    canvas.save();
    canvas.clipPath(head);
    fill(
      Path()
        ..moveTo(38, 56)
        ..lineTo(66, 56)
        ..cubicTo(64, 70, 56, 80, 40, 84)
        ..close(),
      patch.deep,
    );
    fill(
      Path()
        ..moveTo(84, 52)
        ..cubicTo(96, 60, 102, 72, 101, 84)
        ..lineTo(88, 78)
        ..cubicTo(88, 68, 86, 60, 84, 52)
        ..close(),
      patch.fill,
    );
    canvas.restore();
    stroke(head, 3);
    stroke(
      Path()
        ..moveTo(55, 86)
        ..quadraticBezierTo(60, 90, 65, 86)
        ..moveTo(76, 86)
        ..quadraticBezierTo(81, 90, 86, 86),
      2.5,
    );
    final nose = Path()
      ..moveTo(67.5, 95)
      ..lineTo(72.5, 95)
      ..lineTo(70, 98)
      ..close();
    fill(nose, patch.fill);
    stroke(nose, 1.2);
    stroke(
      Path()
        ..moveTo(40, 94)
        ..lineTo(54, 96)
        ..moveTo(40, 100)
        ..lineTo(54, 100)
        ..moveTo(100, 94)
        ..lineTo(86, 96)
        ..moveTo(100, 100)
        ..lineTo(86, 100),
      1.5,
    );
    stroke(
      Path()
        ..moveTo(168, 118)
        ..cubicTo(186, 116, 188, 130, 168, 130)
        ..lineTo(112, 130),
      3,
    );
    for (final (at, font) in [
      (const Offset(104, 44), 16.0),
      (const Offset(116, 30), 12.0),
    ]) {
      final text = TextPainter(
        text: TextSpan(
          text: 'z',
          style: TextStyle(
            fontFamily: sansFamily,
            fontSize: font,
            color: outline,
            height: 1,
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      text.paint(
        canvas,
        Offset(
          at.dx,
          at.dy - text.computeDistanceToActualBaseline(TextBaseline.alphabetic),
        ),
      );
      text.dispose();
    }
  }

  @override
  bool shouldRepaint(SleepingCatPainter old) =>
      old.patch != patch ||
      old.sheet != sheet ||
      old.outline != outline ||
      old.ground != ground;
}

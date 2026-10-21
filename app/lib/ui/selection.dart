// Text selection handles drawn in the accent colour, with no toolbar (the
// app draws its own selection toolbar or keyboard toolbar instead).

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

import 'tokens.dart';

class NoteSelectionControls extends TextSelectionControls {
  static const _r = 7.0;

  @override
  Widget buildHandle(BuildContext context, TextSelectionHandleType type, double textLineHeight,
      [VoidCallback? onTap]) {
    final color = context.colors.accent;
    return SizedBox(
      width: _r * 2 + 8,
      height: textLineHeight + _r * 2,
      child: CustomPaint(painter: _HandlePainter(color, type, textLineHeight)),
    );
  }

  @override
  Offset getHandleAnchor(TextSelectionHandleType type, double textLineHeight) =>
      Offset(_r + 4, type == TextSelectionHandleType.collapsed ? 0 : textLineHeight);

  @override
  Size getHandleSize(double textLineHeight) => Size(_r * 2 + 8, textLineHeight + _r * 2);

  @override
  Widget buildToolbar(
    BuildContext context,
    Rect globalEditableRegion,
    double textLineHeight,
    Offset selectionMidpoint,
    List<TextSelectionPoint> endpoints,
    TextSelectionDelegate delegate,
    ValueListenable<ClipboardStatus>? clipboardStatus,
    Offset? lastSecondaryTapDownPosition,
  ) =>
      const SizedBox.shrink();
}

final noteSelectionControls = NoteSelectionControls();

class _HandlePainter extends CustomPainter {
  _HandlePainter(this.color, this.type, this.lineHeight);
  final Color color;
  final TextSelectionHandleType type;
  final double lineHeight;
  @override
  void paint(Canvas canvas, Size size) {
    final p = Paint()..color = color;
    final cx = size.width / 2;
    // A thin stem through the line and a dot below it, as on mobile
    // handles.
    if (type == TextSelectionHandleType.collapsed) {
      canvas.drawCircle(Offset(cx, 8), 6, p);
      return;
    }
    canvas.drawRect(Rect.fromLTWH(cx - 1, 0, 2, lineHeight), p);
    canvas.drawCircle(Offset(cx, lineHeight + 6), 5, p);
  }

  @override
  bool shouldRepaint(_HandlePainter old) => old.color != color || old.lineHeight != lineHeight;
}

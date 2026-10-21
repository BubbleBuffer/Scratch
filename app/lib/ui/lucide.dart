// Lucide (https://lucide.dev, ISC) drawn from the lucide_icons_flutter font.
// The app's icon enums (Ico in icons.dart, Ix in icon_x.dart) map their names
// to Lucide glyphs here; call sites keep using Icon20/IconX/IconBtn/IconBtnX.
// Source, version and licence: THIRD_PARTY.md at the repo root.

import 'package:flutter/widgets.dart';

/// One Lucide glyph in three stroke weights (1.55, 2 and 2.5 on Lucide's
/// 24-unit grid). Each is a const IconData, so release builds can subset
/// the fonts to the glyphs the app uses.
typedef LucideGlyph = (IconData light, IconData regular, IconData bold);

/// The weight whose stroke at [size] comes closest to [stroke] px, so icons
/// keep the app's line weight at every size (1.5 px at 20 px is Lucide's
/// default weight).
IconData lucideWeight(LucideGlyph g, double size, double stroke) {
  final k = size / 24;
  final light = (1.55 * k - stroke).abs();
  final regular = (2.0 * k - stroke).abs();
  final bold = (2.5 * k - stroke).abs();
  if (light < regular && light < bold) return g.$1;
  if (bold < regular) return g.$3;
  return g.$2;
}

TextStyle _style(IconData d, double size, Color color) => TextStyle(
      inherit: false,
      color: color,
      fontSize: size,
      fontFamily: d.fontFamily,
      package: d.fontPackage,
      height: 1,
      leadingDistribution: TextLeadingDistribution.even,
    );

/// A Lucide glyph, [size] square. [fill] (in Lucide's 24-unit grid) is
/// painted under the outline for the filled variants (a filled star).
class LucideIcon extends StatelessWidget {
  const LucideIcon(this.data, {super.key, required this.size, required this.color, this.fill});
  final IconData data;
  final double size;
  final Color color;
  final List<Offset>? fill;

  @override
  Widget build(BuildContext context) {
    Widget glyph = RichText(
      overflow: TextOverflow.visible,
      textDirection: TextDirection.ltr,
      textScaler: TextScaler.noScaling,
      text: TextSpan(text: String.fromCharCode(data.codePoint), style: _style(data, size, color)),
    );
    if (fill != null) {
      glyph = CustomPaint(painter: _FillPainter(fill!, color), child: glyph);
    }
    return ExcludeSemantics(
      child: SizedBox(width: size, height: size, child: Center(child: glyph)),
    );
  }
}

/// Paints [data] into the square [rect] on [canvas] (for painters that draw
/// icons inline, such as page mentions in the editor).
void paintLucide(Canvas canvas, IconData data, Rect rect, Color color, {List<Offset>? fill}) {
  if (fill != null) {
    canvas.save();
    canvas.translate(rect.left, rect.top);
    _FillPainter(fill, color).paint(canvas, rect.size);
    canvas.restore();
  }
  final tp = TextPainter(
    textDirection: TextDirection.ltr,
    textScaler: TextScaler.noScaling,
    text: TextSpan(text: String.fromCharCode(data.codePoint), style: _style(data, rect.height, color)),
  )..layout();
  tp.paint(canvas, rect.center - Offset(tp.width / 2, tp.height / 2));
  tp.dispose();
}

class _FillPainter extends CustomPainter {
  _FillPainter(this.points, this.color);
  final List<Offset> points;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final k = size.shortestSide / 24;
    final path = Path()..addPolygon([for (final p in points) p * k], true);
    canvas.drawPath(path, Paint()..color = color);
  }

  @override
  bool shouldRepaint(_FillPainter old) => old.color != color || old.points != points;
}

/// The inside of Lucide's `star` outline (the corners of its path).
const lucideStarFill = [
  Offset(12, 2.3), Offset(14.79, 6.97), Offset(16.38, 8.13), Offset(21.55, 8.89),
  Offset(21.84, 9.79), Offset(18.1, 13.43), Offset(17.49, 15.31), Offset(18.38, 20.45),
  Offset(17.6, 21.01), Offset(12.99, 18.58), Offset(11.01, 18.58), Offset(6.4, 21.01),
  Offset(5.63, 20.45), Offset(6.51, 15.31), Offset(5.9, 13.43), Offset(2.16, 9.8),
  Offset(2.45, 8.89), Offset(7.62, 8.13), Offset(9.22, 6.97),
];

/// The inside of Lucide's `play` outline.
const lucidePlayFill = [
  Offset(5, 5), Offset(8.01, 3.27), Offset(20.01, 10.27), Offset(20.01, 13.73),
  Offset(8.01, 20.73), Offset(5, 19),
];

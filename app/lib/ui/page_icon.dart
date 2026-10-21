// A page's icon: its emoji when it has one, else the document glyph.

import 'package:flutter/widgets.dart';

import 'block_colors.dart';
import 'icons.dart';
import 'tokens.dart';

class PageIcon extends StatelessWidget {
  const PageIcon({super.key, this.icon, this.size = 20, this.color});

  /// The page's icon from the core (an emoji), or null.
  final String? icon;
  final double size;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final i = icon;
    if (i != null && i.isNotEmpty) {
      return SizedBox(
        width: size,
        height: size,
        child: Center(child: Text(i, style: TextStyle(fontSize: size * 0.85, height: 1, fontFamilyFallback: emojiFallback))),
      );
    }
    return Icon20(Ico.page, size: size, color: color ?? context.colors.iconSecondary);
  }
}

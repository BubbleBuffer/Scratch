// A page's icon: its emoji when it has one, else the document glyph.

import 'package:flutter/widgets.dart';

import 'block_colors.dart';
import 'icons.dart';
import 'icon_x.dart';
import 'tokens.dart';

class PageIcon extends StatelessWidget {
  const PageIcon({super.key, this.icon, this.size, this.color});

  /// The page's icon from the core (an emoji), or null.
  final String? icon;

  /// The icon's box; [IconSize.lg] by default. Any size, so a space tile can
  /// scale it to its own.
  final double? size;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final size = this.size ?? IconSize.lg.px;
    final i = icon;
    if (i != null && i.isNotEmpty) {
      return SizedBox(
        width: size,
        height: size,
        child: Center(child: Text(i, style: emojiIconStyle(size, emojiFallback))),
      );
    }
    return icoAt(Ico.page, size, color ?? context.colors.iconSecondary);
  }
}

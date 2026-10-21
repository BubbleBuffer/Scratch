import 'package:flutter/widgets.dart';

import 'tokens.dart';
import 'page_icon.dart';

/// The same small patch wherever an item identifies its space.
class SpaceDot extends StatelessWidget {
  const SpaceDot(this.name, {super.key});
  final String? name;

  @override
  Widget build(BuildContext context) {
    final color = spacePatch(
      name,
      context.colors.dark ? Brightness.dark : Brightness.light,
    )?.fill;
    return color == null
        ? const SizedBox.shrink()
        : Container(
            width: 8,
            height: 8,
            decoration: BoxDecoration(
              color: color,
              borderRadius: BorderRadius.circular(2),
            ),
          );
  }
}

/// Emoji remains authored content; document glyphs use the patch ink.
class SpaceIconTile extends StatelessWidget {
  const SpaceIconTile({
    super.key,
    this.icon,
    this.patch,
    this.size = 22,
    this.glyphSize,
    this.color,
  });
  final String? icon;
  final SpacePatch? patch;
  final double size;
  final double? glyphSize;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    if (patch == null) {
      return PageIcon(icon: icon, size: glyphSize ?? size, color: color);
    }
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(color: patch!.fill, borderRadius: Radii.ctl),
      alignment: Alignment.center,
      child: PageIcon(
        icon: icon,
        size: glyphSize ?? size * .7,
        color: patch!.ink,
      ),
    );
  }
}

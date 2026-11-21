// The colour menu: text colours as a coloured "A", then
// background colours as filled squares. Picks a palette name (`red`,
// `red_background`, or `default` / `default_background` to clear).

import 'package:flutter/widgets.dart';

import '../ui/block_colors.dart';
import '../ui/ui.dart';

class ColorMenu extends StatelessWidget {
  const ColorMenu({super.key, this.current, required this.onPick, this.sheet = false});

  /// The current colour, to tick.
  final String? current;
  final ValueChanged<String> onPick;

  /// Rows sized for the sheet (touch).
  final bool sheet;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = sheet || context.touch;
    Widget swatch(String name, bool bg) {
      final base = name == 'default' ? null : name;
      return Container(
        width: m ? EditorMenu.swatchTouch : EditorMenu.swatch,
        height: m ? EditorMenu.swatchTouch : EditorMenu.swatch,
        alignment: Alignment.center,
        decoration: BoxDecoration(
          color: bg ? (base == null ? c.bgPrimary : paletteBackground(base, c)) : null,
          border: Border.all(color: c.border),
          borderRadius: BorderRadius.circular(Radii.sm),
        ),
        child: bg
            ? null
            : Text('A', style: ts(context.type.item, base == null ? c.textPrimary : paletteText(base, c)!)),
      );
    }

    Widget item(String name, bool bg) {
      final value = bg ? '${name}_background' : name;
      final on = (current ?? 'default') == value || (current == null && value == 'default');
      final label = bg ? '${paletteLabel(name)} background' : (name == 'default' ? 'Default text' : paletteLabel(name));
      return Hover(
        key: ValueKey('color-$value'),
        onTap: () => onPick(value),
        radius: Radii.md,
        padding: const EdgeInsets.symmetric(horizontal: Space.sm),
        child: ConstrainedBox(
          constraints: BoxConstraints(minHeight: (m ? ControlSize.xl : ControlSize.sm).height),
          child: Row(children: [
            swatch(name, bg),
            Gap(m ? Space.md : Space.sm),
            Expanded(child: Text(label, style: ts(m ? context.type.body : context.type.ui, c.textPrimary, weight: Weight.regular))),
            if (on) Icon20(Ico.check, size: IconSize.sm, color: c.iconPrimary),
          ]),
        ),
      );
    }

    final children = [
      const MenuHeader('Text color'),
      item('default', false),
      for (final n in paletteNames) item(n, false),
      const MenuHeader('Background color'),
      item('default', true),
      for (final n in paletteNames) item(n, true),
    ];
    if (sheet) {
      return SheetSurface(children: children);
    }
    return MenuSurface(
      width: Widths.popover,
      padding: const EdgeInsets.all(Space.xs),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxHeight: EditorMenu.colorHeight),
        child: SingleChildScrollView(
          child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: children),
        ),
      ),
    );
  }
}

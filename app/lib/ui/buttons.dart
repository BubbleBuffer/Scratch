// Buttons: square icon buttons, the accent button and plain text buttons.

import 'package:flutter/widgets.dart';

import 'hover.dart';
import 'icons.dart';
import 'tokens.dart';
import 'tooltip.dart';

/// A square icon button: [size] hit area with a [glyph]-sized icon.
class IconBtn extends StatelessWidget {
  const IconBtn(
    this.icon, {
    super.key,
    this.onTap,
    this.size = 28,
    this.glyph = 20,
    this.color,
    this.radius = Radii.md,
    this.selected = false,
    this.width,
    this.semanticLabel,
    this.tooltip,
    this.command,
  });
  final Ico icon;
  final VoidCallback? onTap;
  final double size, glyph, radius;
  final double? width;
  final Color? color;
  final bool selected;
  final String? semanticLabel;

  /// Tooltip text; defaults to [semanticLabel]. Buttons without either
  /// get no tooltip.
  final String? tooltip;

  /// The registered command this button runs; the tooltip shows its shortcut.
  final String? command;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final tip = tooltip ?? semanticLabel;
    final button = Semantics(
      button: true,
      label: semanticLabel ?? tooltip ?? icon.name,
      child: Hover(
        onTap: onTap,
        radius: radius,
        selected: selected,
        selectedColor: c.bgPressed,
        child: SizedBox(
          width: width ?? size,
          height: size,
          child: Center(
            child: Icon20(icon, size: glyph, color: color ?? c.iconSecondary),
          ),
        ),
      ),
    );
    return tip == null ? button : AppTooltip(label: tip, command: command, child: button);
  }
}

/// The accent-coloured button (for example "New page" on desktop home).
class PrimaryButton extends StatelessWidget {
  const PrimaryButton({
    super.key,
    required this.label,
    required this.onTap,
    this.height = 42,
    this.icon,
  });
  final String label;
  final VoidCallback onTap;
  final double height;
  final Ico? icon;
  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Semantics(
      button: true,
      child: Hover(
        onTap: onTap,
        borderRadius: Radii.ctl,
        background: c.accent,
        hoverBackground: c.accentHover,
        pressedBackground: c.accentHover,
        padding: const EdgeInsets.symmetric(horizontal: 12),
        child: SizedBox(
          height: height,
          child: Row(
            mainAxisAlignment: MainAxisAlignment.center,
            mainAxisSize: MainAxisSize.min,
            children: [
              if (icon != null) ...[
                Icon20(icon!, size: 18, color: c.onAccent),
                const SizedBox(width: 6),
              ],
              Text(label, style: ts(context.type.ui, c.onAccent, weight: 700)),
            ],
          ),
        ),
      ),
    );
  }
}

/// A sheet-coloured, thread-bordered secondary action.
class SecondaryButton extends StatelessWidget {
  const SecondaryButton({
    super.key,
    required this.label,
    this.onTap,
    this.height = 42,
  });
  final String label;
  final VoidCallback? onTap;
  final double height;
  @override
  Widget build(BuildContext context) => Hover(
    onTap: onTap,
    background: context.colors.bgPrimary,
    border: Border.all(color: context.colors.thread),
    borderRadius: Radii.ctl,
    padding: const EdgeInsets.symmetric(horizontal: 12),
    child: SizedBox(
      height: height,
      child: Center(
        child: Text(
          label,
          style: ts(context.type.ui, context.colors.textPrimary, weight: 600),
        ),
      ),
    ),
  );
}

/// The shared task box used by Today, the editor and database cells.
class StacksCheckbox extends StatelessWidget {
  const StacksCheckbox({
    super.key,
    required this.checked,
    this.size = 22,
    this.onChanged,
    this.fill,
    this.ink,
  });
  final bool checked;
  final double size;
  final ValueChanged<bool>? onChanged;
  final Color? fill, ink;
  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Semantics(
      checked: checked,
      child: Hover(
        onTap: onChanged == null ? null : () => onChanged!(!checked),
        borderRadius: Radii.ctl,
        child: AnimatedContainer(
          duration: Motion.of(context).fast,
          curve: Motion.settle,
          width: size,
          height: size,
          decoration: BoxDecoration(
            color: checked ? (fill ?? c.accent) : null,
            borderRadius: Radii.ctl,
            border: Border.all(
              color: checked ? (fill ?? c.accent) : c.threadStrong,
              width: 1.5,
            ),
          ),
          child: checked
              ? Icon20(Ico.check, size: size * .72, color: ink ?? c.onAccent)
              : null,
        ),
      ),
    );
  }
}

/// A text-only button on a hover surface (for example "Share").
class TextBtn extends StatelessWidget {
  const TextBtn(
    this.label, {
    super.key,
    this.onTap,
    this.height = 28,
    this.color,
  });
  final String label;
  final VoidCallback? onTap;
  final double height;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Hover(
      radius: Radii.md,
      onTap: onTap,
      padding: const EdgeInsets.symmetric(horizontal: 8),
      child: SizedBox(
        height: height,
        child: Center(
          child: Text(
            label,
            style: ts(
              context.type.ui,
              color == c.accent ? c.accentText : (color ?? c.textPrimary),
            ),
          ),
        ),
      ),
    );
  }
}

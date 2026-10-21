// Building blocks of the settings screens:
// section titles with a divider, rows with a label, a description and a
// control on the right, a toggle, a select button, a bordered text field
// and a secondary button. Built from the UI kit's tokens and parts.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';

/// A section title ("Sync") with a hairline below. A [primary] heading
/// names the whole section; on mobile the top bar shows that instead.
class SettingsHeading extends StatelessWidget {
  const SettingsHeading(this.text, {super.key, this.top = 0, this.primary = false});
  final String text;
  final double top;
  final bool primary;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    if (primary && context.mobile) return const SizedBox(height: 4);
    return Padding(
      padding: EdgeInsets.only(top: top),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Text(text, style: primary ? tsDisplay(const TypeSpec(19, 24, 640), c.textPrimary) : tsRaw(16, 24, 500, c.textPrimary)),
        const SizedBox(height: 12),
        HLine(color: c.border),
        const SizedBox(height: 16),
      ]),
    );
  }
}

/// One setting: label and description on the left, [trailing] on the right.
/// On mobile the trailing control goes below when [stack] is set.
class SettingsRow extends StatelessWidget {
  const SettingsRow({super.key, required this.label, this.description, this.trailing, this.stack = false});
  final String label;
  final String? description;
  final Widget? trailing;
  final bool stack;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final text = Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
      Text(label, style: m ? tsRaw(16, 22, 400, c.textPrimary) : tsRaw(14, 20, 400, c.textPrimary)),
      if (description != null) ...[
        const SizedBox(height: 2),
        Text(description!, style: m ? tsRaw(13, 18, 400, c.textSecondary) : tsRaw(12, 16, 400, c.textSecondary)),
      ],
    ]);
    if (trailing == null) return Padding(padding: const EdgeInsets.symmetric(vertical: 8), child: text);
    if (stack) {
      return Padding(
        padding: const EdgeInsets.symmetric(vertical: 8),
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [text, const SizedBox(height: 10), trailing!]),
      );
    }
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Row(children: [
        Expanded(child: text),
        const SizedBox(width: 16),
        trailing!,
      ]),
    );
  }
}

/// Switch: a 30x18 pill, accent when on.
class Toggle extends StatelessWidget {
  const Toggle({super.key, required this.value, required this.onChanged});
  final bool value;
  final ValueChanged<bool>? onChanged;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final off = c.dark ? const Color(0x33FFFFFF) : const Color(0x29000000);
    return Semantics(
      toggled: value,
      button: true,
      child: MouseRegion(
        cursor: onChanged == null ? MouseCursor.defer : SystemMouseCursors.click,
        child: GestureDetector(
          onTap: onChanged == null ? null : () => onChanged!(!value),
          child: AnimatedContainer(
            duration: Motion.of(context).fast,
            width: 30,
            height: 18,
            padding: const EdgeInsets.all(2),
            decoration: BoxDecoration(color: value ? c.accent : off, borderRadius: BorderRadius.circular(9)),
            child: AnimatedAlign(
              duration: Motion.of(context).fast,
              curve: Motion.easing,
              alignment: value ? Alignment.centerRight : Alignment.centerLeft,
              child: Container(
                width: 14,
                height: 14,
                decoration: const BoxDecoration(color: Color(0xFFFFFFFF), shape: BoxShape.circle),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// A value with a chevron that opens a menu of [options] (a popover on
/// desktop, a sheet on mobile).
class SelectButton<T> extends StatelessWidget {
  const SelectButton({
    super.key,
    required this.value,
    required this.options,
    required this.labelOf,
    required this.onChanged,
    this.title,
  });
  final T value;
  final List<T> options;
  final String Function(T) labelOf;
  final ValueChanged<T>? onChanged;
  final String? title;

  Future<void> _open(BuildContext context) async {
    T? picked;
    if (context.mobile) {
      picked = await showSheet<T>(
        context,
        (ctx) => SheetSurface(children: [
          if (title != null) SheetHeader(title!),
          for (final o in options)
            SheetItem(label: labelOf(o), selected: o == value, onTap: () => Navigator.of(ctx).pop(o)),
        ]),
      );
    } else {
      picked = await showPopover<T>(
        context,
        globalRectOf(context),
        (ctx) => MenuList(width: 220, children: [
          for (final o in options)
            MenuItem(
              key: ValueKey('option-${labelOf(o)}'),
              label: labelOf(o),
              onTap: () => Navigator.of(ctx).pop(o),
              trailing: o == value ? Icon20(Ico.check, size: 16, color: ctx.colors.iconPrimary) : null,
            ),
        ]),
      );
    }
    if (picked != null && picked != value) onChanged?.call(picked);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Semantics(
      button: true,
      enabled: onChanged != null,
      child: Builder(
        builder: (b) => Hover(
          onTap: onChanged == null ? null : () => _open(b),
          hoverColor: onChanged == null ? const Color(0x00000000) : null,
          radius: Radii.md,
          padding: const EdgeInsets.fromLTRB(8, 0, 4, 0),
          child: SizedBox(
            height: 28,
            child: Row(mainAxisSize: MainAxisSize.min, children: [
              Text(labelOf(value), style: tsRaw(14, 20, 400, onChanged == null ? c.textDisabled : c.textSecondary)),
              const SizedBox(width: 2),
              Icon20(Ico.chevronDown, size: 14, color: onChanged == null ? c.textDisabled : c.iconTertiary),
            ]),
          ),
        ),
      ),
    );
  }
}

/// A bordered single-line text field (light fill, 1 px
/// border, radius 6).
class SettingsField extends StatefulWidget {
  const SettingsField({
    super.key,
    required this.controller,
    this.placeholder = '',
    this.onSubmitted,
    this.onChanged,
    this.mono = false,
    this.fieldKey,
    this.autofocus = false,
  });
  final TextEditingController controller;
  final String placeholder;
  final ValueChanged<String>? onSubmitted;
  final ValueChanged<String>? onChanged;
  final bool mono;
  final Key? fieldKey;
  final bool autofocus;

  @override
  State<SettingsField> createState() => _SettingsFieldState();
}

class _SettingsFieldState extends State<SettingsField> {
  final _focus = FocusNode();

  @override
  void initState() {
    super.initState();
    _focus.addListener(() => setState(() {}));
  }

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final style = widget.mono
        ? ts(TypeSpec(m ? 14 : 13, 20, 400), c.textPrimary, mono: true)
        : tsRaw(m ? 16 : 14, 20, 400, c.textPrimary);
    return GestureDetector(
      onTap: _focus.requestFocus,
      child: FieldFrame(
        height: m ? 40 : 32,
        padding: const EdgeInsets.symmetric(horizontal: 10,
        ),
        child: PlainField(
          key: widget.fieldKey,
          controller: widget.controller,
          focusNode: _focus,
          style: style,
          placeholder: widget.placeholder,
          onSubmitted: widget.onSubmitted,
          onChanged: widget.onChanged,
          autofocus: widget.autofocus,
          textInputAction: TextInputAction.done,
        ),
      ),
    );
  }
}

/// A secondary button: 1 px border, text in the primary colour.
class OutlineButton extends StatelessWidget {
  const OutlineButton({super.key, required this.label, required this.onTap, this.danger = false, this.icon});
  final String label;
  final VoidCallback? onTap;
  final bool danger;
  final Widget? icon;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final color = onTap == null ? c.textDisabled : (danger ? c.inlineCodeText : c.textPrimary);
    return Hover(
      onTap: onTap,
      radius: Radii.md,
      child: Container(
        height: 32,
        padding: const EdgeInsets.symmetric(horizontal: 12),
        decoration: BoxDecoration(
          color: c.bgPrimary,
          borderRadius:Radii.ctl,
          border: Border.all(color: danger ? c.inlineCodeText.withValues(alpha: 0.5) : c.thread),
        ),
        child: Row(mainAxisSize: MainAxisSize.min, children: [
          if (icon != null) ...[icon!, const SizedBox(width: 6)],
          Text(label, style: tsRaw(14, 20, 500, color)),
        ]),
      ),
    );
  }
}

/// Monospace text that can be copied, with a "Copy" button.
class CopyableText extends StatefulWidget {
  const CopyableText(this.text, {super.key, this.textKey});
  final String text;
  final Key? textKey;

  @override
  State<CopyableText> createState() => _CopyableTextState();
}

class _CopyableTextState extends State<CopyableText> {
  bool _copied = false;

  Future<void> _copy() async {
    await Clipboard.setData(ClipboardData(text: widget.text));
    if (!mounted) return;
    setState(() => _copied = true);
    await Future<void>.delayed(const Duration(seconds: 2));
    if (mounted) setState(() => _copied = false);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      padding: const EdgeInsets.fromLTRB(10, 6, 4, 6),
      decoration: BoxDecoration(color: c.bgSecondary, borderRadius:Radii.field, border: Border.all(color: c.borderSubtle)),
      child: Row(crossAxisAlignment: CrossAxisAlignment.center, children: [
        Expanded(
          child: Text(widget.text,
              key: widget.textKey, style: ts(const TypeSpec(13, 19, 400), c.textPrimary, mono: true)),
        ),
        const SizedBox(width: 8),
        TextBtn(_copied ? 'Copied' : 'Copy', onTap: _copy, color: c.textSecondary),
      ]),
    );
  }
}

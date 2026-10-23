// The Settings kit: the parts every settings page is built from, matching
// the settings redesign mockup (design/settings-redesign/settings.css).
// Pages, sections, rows with an always-visible description, ⓘ help with a
// "When to change it" note, lock lines for options that depend on another,
// the square switch, buttons, selects, segmented controls, choice cards,
// lists with empty states, notices, the status card, advanced disclosures,
// copy lines and the collapsible Connectors panels.
//
// Other screens keep the older controls.dart parts (Toggle, SettingsRow);
// this kit is for Settings only.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../ui/lucide.dart';
import '../../ui/ui.dart';

// ---------------------------------------------------------------------------
// Colours the mockup uses that are not app tokens.

extension SettingsColors on AppColors {
  /// The switch's "on" colour (the moss space colour).
  Color get moss => dark ? const Color(0xFF8FA392) : const Color(0xFF697A6B);

  /// Text for destructive actions and errors.
  Color get dangerText => dark ? const Color(0xFFF2B8B0) : const Color(0xFF8A3027);

  /// The left rule of a warning notice.
  Color get warnRule => const Color(0xFFC9A24C);
}

// ---------------------------------------------------------------------------
// Icons (Lucide, ISC; see THIRD_PARTY.md).

enum Sx {
  general((LucideIcons.slidersHorizontal300, LucideIcons.slidersHorizontal, LucideIcons.slidersHorizontal500)),
  reminders((LucideIcons.bell300, LucideIcons.bell, LucideIcons.bell500)),
  sync((LucideIcons.refreshCw300, LucideIcons.refreshCw, LucideIcons.refreshCw500)),
  conflicts((LucideIcons.gitCompare300, LucideIcons.gitCompare, LucideIcons.gitCompare500)),
  storage((LucideIcons.hardDrive300, LucideIcons.hardDrive, LucideIcons.hardDrive500)),
  agent((LucideIcons.bot300, LucideIcons.bot, LucideIcons.bot500)),
  connectors((LucideIcons.plug300, LucideIcons.plug, LucideIcons.plug500)),
  info((LucideIcons.info300, LucideIcons.info, LucideIcons.info500)),
  lock((LucideIcons.lock300, LucideIcons.lock, LucideIcons.lock500)),
  chevronRight((LucideIcons.chevronRight300, LucideIcons.chevronRight, LucideIcons.chevronRight500)),
  chevronDown((LucideIcons.chevronDown300, LucideIcons.chevronDown, LucideIcons.chevronDown500)),
  chevronLeft((LucideIcons.chevronLeft300, LucideIcons.chevronLeft, LucideIcons.chevronLeft500)),
  wrench((LucideIcons.wrench300, LucideIcons.wrench, LucideIcons.wrench500)),
  appWindow((LucideIcons.appWindow300, LucideIcons.appWindow, LucideIcons.appWindow500)),
  monitor((LucideIcons.monitor300, LucideIcons.monitor, LucideIcons.monitor500)),
  server((LucideIcons.server300, LucideIcons.server, LucideIcons.server500)),
  plus((LucideIcons.plus300, LucideIcons.plus, LucideIcons.plus500)),
  pencil((LucideIcons.pencil300, LucideIcons.pencil, LucideIcons.pencil500)),
  trash((LucideIcons.trash2300, LucideIcons.trash2, LucideIcons.trash2500)),
  terminal((LucideIcons.terminal300, LucideIcons.terminal, LucideIcons.terminal500)),
  globe((LucideIcons.globe300, LucideIcons.globe, LucideIcons.globe500)),
  key((LucideIcons.keyRound300, LucideIcons.keyRound, LucideIcons.keyRound500)),
  sparkles((LucideIcons.sparkles300, LucideIcons.sparkles, LucideIcons.sparkles500)),
  folder((LucideIcons.folder300, LucideIcons.folder, LucideIcons.folder500)),
  home((LucideIcons.house300, LucideIcons.house, LucideIcons.house500)),
  check((LucideIcons.check300, LucideIcons.check, LucideIcons.check500)),
  alert((LucideIcons.circleAlert300, LucideIcons.circleAlert, LucideIcons.circleAlert500)),
  warn((LucideIcons.triangleAlert300, LucideIcons.triangleAlert, LucideIcons.triangleAlert500)),
  laptop((LucideIcons.laptop300, LucideIcons.laptop, LucideIcons.laptop500)),
  phone((LucideIcons.smartphone300, LucideIcons.smartphone, LucideIcons.smartphone500));

  const Sx(this.glyph);
  final LucideGlyph glyph;
}

class SIcon extends StatelessWidget {
  const SIcon(this.icon, {super.key, this.size = 16, required this.color, this.stroke = 1.6});
  final Sx icon;
  final double size;
  final Color color;
  final double stroke;

  @override
  Widget build(BuildContext context) =>
      LucideIcon(lucideWeight(icon.glyph, size, stroke), size: size, color: color);
}

// ---------------------------------------------------------------------------
// Text styles.

TextStyle sLabel(BuildContext context, {Color? color}) =>
    tsRaw(14.5, 20, 640, color ?? context.colors.textPrimary);
TextStyle sDesc(BuildContext context, {Color? color}) =>
    tsRaw(13.5, 19, 400, color ?? context.colors.textSecondary);
TextStyle sHint(BuildContext context) => tsRaw(12.5, 17, 400, context.colors.textTertiary);
TextStyle sMono(BuildContext context, {Color? color}) =>
    ts(const TypeSpec(12.5, 18, 400), color ?? context.colors.textPrimary, mono: true);

/// Turns `**bold**` in [text] into bold spans, for descriptions and intros.
InlineSpan sRich(String text, TextStyle style) {
  final parts = text.split('**');
  if (parts.length == 1) return TextSpan(text: text, style: style);
  return TextSpan(style: style, children: [
    for (var i = 0; i < parts.length; i++)
      TextSpan(text: parts[i], style: i.isOdd ? style.copyWith(fontWeight: FontWeight.w700) : null),
  ]);
}

// ---------------------------------------------------------------------------
// Page and section.

/// A settings page: title, intro, then its sections. On a phone the title is
/// shown here too, under the "‹ Settings" crumb.
class SPage extends StatelessWidget {
  const SPage({super.key, required this.title, this.intro, required this.children});
  final String title;
  final String? intro;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Semantics(
        header: true,
        child: Text(title, style: tsDisplay(m ? const TypeSpec(26, 30, 640) : const TypeSpec(28, 32, 640), c.textPrimary)),
      ),
      if (intro != null) ...[
        const SizedBox(height: 8),
        ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 560),
          child: Align(
            alignment: Alignment.centerLeft,
            child: Text.rich(sRich(intro!, tsRaw(15, 22, 400, c.textSecondary))),
          ),
        ),
      ],
      ...children,
    ]);
  }
}

/// A titled group of rows. [first] drops the top margin.
class SSection extends StatelessWidget {
  const SSection({super.key, this.title, this.intro, required this.children, this.first = false});
  final String? title;
  final String? intro;
  final List<Widget> children;
  final bool first;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Padding(
      padding: EdgeInsets.only(top: first ? 18 : 28),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        if (title != null)
          Container(
            padding: const EdgeInsets.only(bottom: 8),
            decoration: BoxDecoration(border: Border(bottom: BorderSide(color: c.border))),
            child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
              Semantics(header: true, child: Text(title!, style: tsDisplay(const TypeSpec(17, 22, 620), c.textPrimary, weight: 620))),
              if (intro != null) ...[
                const SizedBox(height: 3),
                Text.rich(sRich(intro!, sDesc(context))),
              ],
            ]),
          ),
        ..._separated(children, c.border),
      ]),
    );
  }
}

List<Widget> _separated(List<Widget> rows, Color line) {
  final out = <Widget>[];
  for (var i = 0; i < rows.length; i++) {
    final last = i == rows.length - 1;
    final r = rows[i];
    out.add(r is SRow && !last
        ? DecoratedBox(decoration: BoxDecoration(border: Border(bottom: BorderSide(color: line))), child: r)
        : r);
  }
  return out;
}

// ---------------------------------------------------------------------------
// Row with help.

/// One option: label, ⓘ help, scope chip, an always-visible description,
/// a lock line when it depends on another option, and its control.
/// [below] holds wider content under the text (a list, fields, a copy line).
class SRow extends StatefulWidget {
  const SRow({
    super.key,
    required this.label,
    this.desc,
    this.help,
    this.extra,
    this.reason,
    this.reasonAction,
    this.reasonActionLabel,
    this.scope,
    this.disabled = false,
    this.control,
    this.below,
    this.stack = false,
  });
  final String label;
  final String? desc;

  /// The "When to change it:" note behind ⓘ.
  final String? help;

  /// A second paragraph in the note.
  final String? extra;

  /// The lock line ("Needs “Everything” above.").
  final String? reason;
  final VoidCallback? reasonAction;
  final String? reasonActionLabel;

  /// A small chip after the label ("Phone", "Desktop", "All devices").
  final String? scope;
  final bool disabled;
  final Widget? control;
  final Widget? below;

  /// Put the control under the text instead of on the right.
  final bool stack;

  @override
  State<SRow> createState() => _SRowState();
}

class _SRowState extends State<SRow> {
  bool _pinned = false, _peek = false, _onNote = false;
  Timer? _close;

  @override
  void dispose() {
    _close?.cancel();
    super.dispose();
  }

  void _show() {
    _close?.cancel();
    if (!_peek) setState(() => _peek = true);
  }

  void _hideSoon() {
    _close?.cancel();
    _close = Timer(const Duration(milliseconds: 250), () {
      if (mounted && !_onNote) setState(() => _peek = false);
    });
  }

  @override
  Widget build(BuildContext context) {
    final w = widget;
    final open = _pinned || _peek;
    final fade = w.disabled ? .62 : 1.0;
    final head = Wrap(crossAxisAlignment: WrapCrossAlignment.center, spacing: 6, runSpacing: 4, children: [
      Opacity(opacity: fade, child: Text(w.label, style: sLabel(context))),
      if (w.help != null)
        _HelpButton(
          label: w.label,
          open: open,
          onTap: () => setState(() {
            _pinned = !open;
            _peek = false;
          }),
          onEnter: _show,
          onExit: _hideSoon,
        ),
      if (w.scope != null) SScope(w.scope!),
    ]);
    final text = Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
      head,
      if (w.desc != null) ...[
        const SizedBox(height: 3),
        Opacity(
          opacity: fade,
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 520),
            child: Text.rich(sRich(w.desc!, sDesc(context))),
          ),
        ),
      ],
      if (open && w.help != null)
        MouseRegion(
          onEnter: (_) {
            _onNote = true;
            _close?.cancel();
          },
          onExit: (_) {
            _onNote = false;
            if (!_pinned) _hideSoon();
          },
          child: _HelpNote(help: w.help!, extra: w.extra),
        ),
      if (w.reason != null) ...[
        const SizedBox(height: 6),
        SReason(w.reason!, action: w.reasonAction, actionLabel: w.reasonActionLabel),
      ],
    ]);
    Widget body;
    if (w.control == null) {
      body = text;
    } else if (w.stack) {
      body = Column(crossAxisAlignment: CrossAxisAlignment.start, children: [text, const SizedBox(height: 10), w.control!]);
    } else {
      body = Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Expanded(child: text),
        const SizedBox(width: 24),
        w.control!,
      ]);
    }
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 14),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        body,
        if (w.below != null) ...[const SizedBox(height: 10), w.below!],
      ]),
    );
  }
}

class _HelpButton extends StatefulWidget {
  const _HelpButton({required this.label, required this.open, required this.onTap, required this.onEnter, required this.onExit});
  final String label;
  final bool open;
  final VoidCallback onTap, onEnter, onExit;

  @override
  State<_HelpButton> createState() => _HelpButtonState();
}

class _HelpButtonState extends State<_HelpButton> {
  final _focus = FocusNode();

  @override
  void initState() {
    super.initState();
    _focus.addListener(() => _focus.hasFocus ? widget.onEnter() : widget.onExit());
  }

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final s = context.mobile ? 28.0 : 22.0;
    return Semantics(
      button: true,
      expanded: widget.open,
      label: 'When to change ${widget.label}',
      child: Focus(
        focusNode: _focus,
        onKeyEvent: (_, e) {
          if (e is KeyDownEvent && (e.logicalKey == LogicalKeyboardKey.enter || e.logicalKey == LogicalKeyboardKey.space)) {
            widget.onTap();
            return KeyEventResult.handled;
          }
          return KeyEventResult.ignored;
        },
        child: MouseRegion(
          cursor: SystemMouseCursors.click,
          onEnter: (_) => widget.onEnter(),
          onExit: (_) => widget.onExit(),
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: widget.onTap,
            child: Container(
              width: s,
              height: s,
              decoration: BoxDecoration(
                color: widget.open ? c.bgSecondary : null,
                borderRadius: BorderRadius.circular(5),
                border: _focus.hasFocus ? Border.all(color: c.focus, width: 2) : null,
              ),
              alignment: Alignment.center,
              child: SIcon(Sx.info, size: 15, color: widget.open ? c.textPrimary : c.textTertiary),
            ),
          ),
        ),
      ),
    );
  }
}

class _HelpNote extends StatelessWidget {
  const _HelpNote({required this.help, this.extra});
  final String help;
  final String? extra;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final style = tsRaw(13, 19, 400, c.textSecondary);
    return Container(
      margin: const EdgeInsets.only(top: 8),
      constraints: const BoxConstraints(maxWidth: 540),
      padding: const EdgeInsets.fromLTRB(12, 8, 12, 9),
      decoration: BoxDecoration(
        color: c.bgSecondary,
        border: Border(left: BorderSide(color: c.moss, width: 3)),
        borderRadius: const BorderRadius.only(
          topLeft: Radius.circular(2),
          topRight: Radius.circular(8),
          bottomRight: Radius.circular(8),
          bottomLeft: Radius.circular(2),
        ),
      ),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text.rich(TextSpan(style: style, children: [
          TextSpan(text: 'When to change it: ', style: style.copyWith(fontWeight: FontWeight.w700, color: c.textPrimary)),
          TextSpan(text: help),
        ])),
        if (extra != null) ...[const SizedBox(height: 6), Text(extra!, style: style)],
      ]),
    );
  }
}

/// A lock line: why an option can't be changed, with an optional link.
class SReason extends StatelessWidget {
  const SReason(this.text, {super.key, this.action, this.actionLabel});
  final String text;
  final VoidCallback? action;
  final String? actionLabel;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final style = tsRaw(13, 18, 600, c.accentText);
    return Row(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
      Padding(padding: const EdgeInsets.only(top: 2), child: SIcon(Sx.lock, size: 13, color: c.accentText)),
      const SizedBox(width: 6),
      Flexible(
        child: Wrap(crossAxisAlignment: WrapCrossAlignment.center, spacing: 8, children: [
          Text(text, style: style),
          if (action != null && actionLabel != null) SLink(actionLabel!, onTap: action!),
        ]),
      ),
    ]);
  }
}

/// A small outlined chip naming where an option applies.
class SScope extends StatelessWidget {
  const SScope(this.text, {super.key});
  final String text;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 1),
      decoration: BoxDecoration(borderRadius: Radii.pill, border: Border.all(color: c.border)),
      child: Text(text, style: tsRaw(11.5, 16, 500, c.textTertiary)),
    );
  }
}

/// An underlined accent link.
class SLink extends StatelessWidget {
  const SLink(this.label, {super.key, required this.onTap});
  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Semantics(
      link: true,
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        child: GestureDetector(
          onTap: onTap,
          child: Text(label,
              style: tsRaw(13, 18, 600, c.accentText).copyWith(
                decoration: TextDecoration.underline,
                decorationColor: c.accentText,
              )),
        ),
      ),
    );
  }
}

/// A line of text with a link in it ("MCP servers are on Connectors.").
class SLinkLine extends StatelessWidget {
  const SLinkLine({super.key, required this.before, required this.link, this.after = '', required this.onTap});
  final String before, link, after;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 12),
      child: Wrap(crossAxisAlignment: WrapCrossAlignment.center, children: [
        Text(before, style: sDesc(context)),
        SLink(link, onTap: onTap),
        if (after.isNotEmpty) Text(after, style: sDesc(context)),
      ]),
    );
  }
}

// ---------------------------------------------------------------------------
// Switch.

/// The square family switch: 40x22 (46x26 on a phone), moss when on.
class SSwitch extends StatelessWidget {
  const SSwitch({super.key, required this.value, required this.onChanged, this.label});
  final bool value;
  final ValueChanged<bool>? onChanged;
  final String? label;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final w = m ? 46.0 : 40.0, h = m ? 26.0 : 22.0, k = m ? 20.0 : 16.0;
    final travel = m ? 20.0 : 18.0;
    final motion = Motion.of(context);
    final on = value;
    return Semantics(
      toggled: on,
      enabled: onChanged != null,
      label: label,
      child: MouseRegion(
        cursor: onChanged == null ? MouseCursor.defer : SystemMouseCursors.click,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: onChanged == null ? null : () => onChanged!(!on),
          child: Opacity(
            opacity: onChanged == null ? .45 : 1,
            child: AnimatedContainer(
              duration: motion.fast,
              width: w,
              height: h,
              decoration: BoxDecoration(
                color: on ? c.moss : c.bgSecondary,
                borderRadius: BorderRadius.circular(5),
                border: Border.all(color: on ? c.moss : c.borderStrong),
              ),
              child: Stack(children: [
                AnimatedPositioned(
                  duration: motion.fast,
                  curve: Motion.easing,
                  left: on ? 2 + travel : 2,
                  top: (h - 2 - k) / 2,
                  child: Container(
                    width: k,
                    height: k,
                    decoration: BoxDecoration(
                      color: c.bgPrimary,
                      border: Border.all(color: on ? c.moss : c.borderStrong),
                      borderRadius: const BorderRadius.only(
                        topLeft: Radius.circular(3),
                        topRight: Radius.circular(4),
                        bottomRight: Radius.circular(3),
                        bottomLeft: Radius.circular(4),
                      ),
                    ),
                  ),
                ),
              ]),
            ),
          ),
        ),
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// Buttons.

enum SButtonKind { normal, primary, quiet, danger }

class SButton extends StatelessWidget {
  const SButton(this.label, {super.key, required this.onTap, this.kind = SButtonKind.normal, this.icon});
  final String label;
  final VoidCallback? onTap;
  final SButtonKind kind;
  final Sx? icon;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final off = onTap == null;
    final fg = switch (kind) {
      SButtonKind.primary => const Color(0xFF282A2F),
      SButtonKind.quiet => c.textSecondary,
      SButtonKind.danger => c.dangerText,
      SButtonKind.normal => c.textPrimary,
    };
    final bg = switch (kind) {
      SButtonKind.primary => c.accent,
      SButtonKind.quiet => null,
      _ => c.bgPrimary,
    };
    return Semantics(
      button: true,
      enabled: !off,
      child: Opacity(
        opacity: off ? .5 : 1,
        child: Hover(
          onTap: onTap,
          borderRadius: Radii.ctl,
          background: bg,
          hoverBackground: kind == SButtonKind.primary ? c.accentHover : c.bgHover,
          border: kind == SButtonKind.quiet || kind == SButtonKind.primary ? null : Border.all(color: c.borderStrong),
          lift: 0,
          child: ConstrainedBox(
            constraints: BoxConstraints(minHeight: context.mobile ? 40 : 34),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 13, vertical: 6),
              child: Row(mainAxisSize: MainAxisSize.min, children: [
                if (icon != null) ...[SIcon(icon!, size: 15, color: fg), const SizedBox(width: 6)],
                Text(label, style: tsRaw(13.5, 18, 680, fg)),
              ]),
            ),
          ),
        ),
      ),
    );
  }
}

class SIconButton extends StatelessWidget {
  const SIconButton(this.icon, {super.key, required this.onTap, required this.label, this.danger = false});
  final Sx icon;
  final VoidCallback? onTap;
  final String label;
  final bool danger;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final s = context.mobile ? 40.0 : 32.0;
    return Semantics(
      button: true,
      label: label,
      enabled: onTap != null,
      child: Opacity(
        opacity: onTap == null ? .4 : 1,
        child: Hover(
          onTap: onTap,
          borderRadius: Radii.ctl,
          lift: 0,
          child: SizedBox(
            width: s,
            height: s,
            child: Center(child: SIcon(icon, size: 16, color: danger ? c.dangerText : c.textSecondary)),
          ),
        ),
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// Select, segmented, choices.

/// A bordered select: a popover on desktop, a sheet on a phone. [value]
/// need not be one of [options] (a saved model that is gone, say).
class SSelect<T> extends StatelessWidget {
  const SSelect({
    super.key,
    required this.value,
    required this.options,
    required this.labelOf,
    required this.onChanged,
    this.title,
    this.minWidth = 180,
  });
  final T value;
  final List<T> options;
  final String Function(T) labelOf;
  final ValueChanged<T>? onChanged;
  final String? title;
  final double minWidth;

  Future<void> _open(BuildContext context) async {
    T? picked;
    if (context.mobile) {
      picked = await showSheet<T>(
        context,
        (ctx) => SheetSurface(children: [
          if (title != null) SheetHeader(title!),
          for (final o in options)
            SheetItem(
              key: ValueKey('option-${labelOf(o)}'),
              label: labelOf(o),
              selected: o == value,
              onTap: () => Navigator.of(ctx).pop(o),
            ),
        ]),
      );
    } else {
      final rect = globalRectOf(context);
      picked = await showPopover<T>(
        context,
        rect,
        (ctx) => MenuList(width: rect.width < 220 ? 220 : rect.width, children: [
          for (final o in options)
            MenuItem(
              key: ValueKey('option-${labelOf(o)}'),
              label: labelOf(o),
              onTap: () => Navigator.of(ctx).pop(o),
              trailing: o == value ? SIcon(Sx.check, size: 15, color: ctx.colors.iconPrimary) : null,
            ),
        ]),
      );
    }
    if (picked != null && picked != value) onChanged?.call(picked);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final off = onChanged == null;
    return Semantics(
      button: true,
      enabled: !off,
      child: Builder(
        builder: (b) => Opacity(
          opacity: off ? .55 : 1,
          child: Hover(
            onTap: off ? null : () => _open(b),
            borderRadius: Radii.field,
            background: c.bgPrimary,
            border: Border.all(color: c.borderStrong),
            lift: 0,
            child: ConstrainedBox(
              constraints: BoxConstraints(minHeight: context.mobile ? 40 : 34, minWidth: minWidth),
              child: Padding(
                padding: const EdgeInsets.fromLTRB(11, 7, 8, 7),
                child: Row(mainAxisSize: MainAxisSize.min, children: [
                  Flexible(child: Text(labelOf(value), overflow: TextOverflow.ellipsis, style: tsRaw(13.5, 18, 500, c.textPrimary))),
                  const SizedBox(width: 8),
                  SIcon(Sx.chevronDown, size: 15, color: c.textTertiary),
                ]),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// A row of mutually exclusive segments.
class SSegmented<T> extends StatelessWidget {
  const SSegmented({super.key, required this.value, required this.options, required this.labelOf, required this.onChanged, this.keyOf});
  final T value;
  final List<T> options;
  final String Function(T) labelOf;
  final ValueChanged<T>? onChanged;
  final Key Function(T)? keyOf;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      padding: const EdgeInsets.all(2),
      decoration: BoxDecoration(color: c.bgSecondary, borderRadius: Radii.ctl, border: Border.all(color: c.borderStrong)),
      child: Row(mainAxisSize: MainAxisSize.min, children: [
        for (final o in options)
          Semantics(
            button: true,
            selected: o == value,
            child: Hover(
              key: keyOf?.call(o),
              onTap: onChanged == null || o == value ? null : () => onChanged!(o),
              borderRadius: Radii.ctl,
              lift: 0,
              background: o == value ? c.bgPrimary : null,
              border: o == value ? Border(bottom: BorderSide(color: c.accent, width: 2)) : null,
              padding: EdgeInsets.symmetric(horizontal: 12, vertical: context.mobile ? 9 : 6),
              child: Text(labelOf(o), style: tsRaw(13.5, 18, o == value ? 680 : 500, o == value ? c.textPrimary : c.textSecondary)),
            ),
          ),
      ]),
    );
  }
}

/// One choice card: title with a radio on the right, a description below.
class SChoice<T> {
  const SChoice(this.value, {required this.title, required this.desc, this.icon, this.disabledNote});
  final T value;
  final String title, desc;
  final Sx? icon;

  /// When set, the card can't be picked and shows this line instead.
  final String? disabledNote;
}

/// A grid of choice cards (two or three across, one on a phone).
class SChoices<T> extends StatelessWidget {
  const SChoices({super.key, required this.value, required this.choices, required this.onChanged, this.keyOf});
  final T? value;
  final List<SChoice<T>> choices;
  final ValueChanged<T>? onChanged;
  final Key Function(T)? keyOf;

  @override
  Widget build(BuildContext context) {
    final cards = [for (final ch in choices) _ChoiceCard<T>(choice: ch, checked: ch.value == value, onChanged: onChanged, cardKey: keyOf?.call(ch.value))];
    if (context.mobile) {
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        for (var i = 0; i < cards.length; i++) ...[if (i > 0) const SizedBox(height: 8), cards[i]],
      ]);
    }
    return IntrinsicHeight(
      child: Row(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        for (var i = 0; i < cards.length; i++) ...[if (i > 0) const SizedBox(width: 10), Expanded(child: cards[i])],
      ]),
    );
  }
}

class _ChoiceCard<T> extends StatelessWidget {
  const _ChoiceCard({required this.choice, required this.checked, required this.onChanged, this.cardKey});
  final SChoice<T> choice;
  final bool checked;
  final ValueChanged<T>? onChanged;
  final Key? cardKey;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final off = choice.disabledNote != null || onChanged == null;
    return Semantics(
      button: true,
      selected: checked,
      enabled: !off,
      child: Opacity(
        opacity: choice.disabledNote != null ? .62 : 1,
        child: Hover(
          key: cardKey,
          onTap: off || checked ? null : () => onChanged!(choice.value),
          borderRadius: Radii.field,
          background: c.bgPrimary,
          lift: 0,
          border: Border.all(color: checked ? c.accent : c.borderStrong, width: checked ? 2 : 1),
          padding: EdgeInsets.fromLTRB(checked ? 13 : 14, checked ? 11 : 12, checked ? 13 : 14, checked ? 11 : 12),
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Row(children: [
              if (choice.icon != null) ...[SIcon(choice.icon!, size: 16, color: c.textSecondary), const SizedBox(width: 8)],
              Expanded(child: Text(choice.title, style: tsRaw(14, 19, 700, c.textPrimary))),
              const SizedBox(width: 8),
              Container(
                width: 16,
                height: 16,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  color: c.bgPrimary,
                  border: Border.all(color: checked ? c.accent : c.borderStrong, width: checked ? 5 : 1.5),
                ),
              ),
            ]),
            const SizedBox(height: 4),
            Text(choice.disabledNote ?? choice.desc, style: tsRaw(13, 18, 400, c.textSecondary)),
          ]),
        ),
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// Fields.

/// A labelled field with an optional hint under it.
class SField extends StatelessWidget {
  const SField({super.key, required this.label, required this.child, this.hint});
  final String label;
  final Widget child;
  final String? hint;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Padding(
      padding: const EdgeInsets.only(top: 10),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Text(label, style: tsRaw(13, 18, 640, c.textPrimary)),
        const SizedBox(height: 4),
        child,
        if (hint != null) ...[const SizedBox(height: 4), Text(hint!, style: sHint(context))],
      ]),
    );
  }
}

/// A bordered single-line text input.
class STextField extends StatefulWidget {
  const STextField({
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
  final ValueChanged<String>? onSubmitted, onChanged;
  final bool mono;
  final Key? fieldKey;
  final bool autofocus;

  @override
  State<STextField> createState() => _STextFieldState();
}

class _STextFieldState extends State<STextField> {
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
    final style = widget.mono ? sMono(context) : tsRaw(m ? 15 : 14, 20, 400, c.textPrimary);
    return GestureDetector(
      onTap: _focus.requestFocus,
      child: Container(
        constraints: BoxConstraints(minHeight: m ? 40 : 36),
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
        decoration: BoxDecoration(
          color: c.bgPrimary,
          borderRadius: Radii.field,
          border: Border.all(color: _focus.hasFocus ? c.focus : c.borderStrong, width: _focus.hasFocus ? 2 : 1),
        ),
        alignment: Alignment.centerLeft,
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

/// A bordered box for adding or editing an item: title, fields, buttons.
class SEditor extends StatelessWidget {
  const SEditor({super.key, required this.title, required this.children, required this.actions, this.desc});
  final String title;
  final String? desc;
  final List<Widget> children;
  final List<Widget> actions;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      margin: const EdgeInsets.only(top: 10),
      padding: const EdgeInsets.fromLTRB(16, 14, 16, 16),
      decoration: BoxDecoration(color: c.bgPrimary, borderRadius: Radii.field, border: Border.all(color: c.accent)),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Text(title, style: tsRaw(14, 19, 700, c.textPrimary)),
        if (desc != null) ...[const SizedBox(height: 3), Text(desc!, style: sDesc(context))],
        ...children,
        const SizedBox(height: 14),
        Wrap(alignment: WrapAlignment.end, spacing: 8, runSpacing: 8, children: actions),
      ]),
    );
  }
}

// ---------------------------------------------------------------------------
// Lists.

class SList extends StatelessWidget {
  const SList({super.key, required this.children, this.foot});
  final List<Widget> children;
  final Widget? foot;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      decoration: BoxDecoration(color: c.bgPrimary, borderRadius: Radii.field, border: Border.all(color: c.border)),
      clipBehavior: Clip.antiAlias,
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        for (var i = 0; i < children.length; i++)
          i == 0
              ? children[i]
              : DecoratedBox(decoration: BoxDecoration(border: Border(top: BorderSide(color: c.border))), child: children[i]),
        if (foot != null)
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
            decoration: BoxDecoration(color: c.bgSecondary, border: children.isEmpty ? null : Border(top: BorderSide(color: c.border))),
            child: foot,
          ),
      ]),
    );
  }
}

/// One list item: a glyph tile, name with tags, a sub line, an error line,
/// and actions on the right.
class SItem extends StatelessWidget {
  const SItem({super.key, this.icon, required this.name, this.tags = const [], this.sub, this.subMono = false, this.error, this.status, this.trailing = const [], this.nameWidget});
  final Sx? icon;
  final String name;
  final Widget? nameWidget;
  final List<String> tags;
  final String? sub;
  final bool subMono;
  final String? error;
  final Widget? status;
  final List<Widget> trailing;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 10, 8, 10),
      child: Row(children: [
        if (icon != null) ...[
          Container(
            width: 32,
            height: 32,
            decoration: BoxDecoration(
              color: c.bgSecondary,
              borderRadius: const BorderRadius.only(
                topLeft: Radius.circular(6),
                topRight: Radius.circular(9),
                bottomRight: Radius.circular(6),
                bottomLeft: Radius.circular(8),
              ),
            ),
            alignment: Alignment.center,
            child: SIcon(icon!, size: 16, color: c.textSecondary),
          ),
          const SizedBox(width: 12),
        ],
        Expanded(
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
            Wrap(crossAxisAlignment: WrapCrossAlignment.center, spacing: 6, runSpacing: 2, children: [
              nameWidget ?? Text(name, style: tsRaw(14, 19, 680, c.textPrimary)),
              for (final t in tags) STag(t),
              ?status,
            ]),
            if (sub != null && sub!.isNotEmpty) ...[
              const SizedBox(height: 2),
              Text(sub!, maxLines: 1, overflow: TextOverflow.ellipsis, style: subMono ? sMono(context, color: c.textTertiary) : sHint(context)),
            ],
            if (error != null) ...[
              const SizedBox(height: 2),
              Text(error!, style: tsRaw(12.5, 17, 500, c.dangerText)),
            ],
          ]),
        ),
        if (trailing.isNotEmpty) ...[
          const SizedBox(width: 12),
          Row(mainAxisSize: MainAxisSize.min, children: [
            for (var i = 0; i < trailing.length; i++) ...[if (i > 0) const SizedBox(width: 4), trailing[i]],
          ]),
        ],
      ]),
    );
  }
}

/// The foot of a list: an add button and an optional note.
class SListFoot extends StatelessWidget {
  const SListFoot({super.key, required this.children});
  final List<Widget> children;

  @override
  Widget build(BuildContext context) => Wrap(crossAxisAlignment: WrapCrossAlignment.center, spacing: 12, runSpacing: 6, children: children);
}

enum SStatusKind { ok, info, warn, bad, off }

/// A status pill with a dot.
class SStatus extends StatelessWidget {
  const SStatus(this.label, {super.key, this.kind = SStatusKind.off});
  final String label;
  final SStatusKind kind;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final bg = switch (kind) {
      SStatusKind.ok => c.okBg,
      SStatusKind.info => c.infoBg,
      SStatusKind.warn => c.attnBg,
      SStatusKind.bad => c.dangerBg,
      SStatusKind.off => c.bgSecondary,
    };
    final dot = switch (kind) {
      SStatusKind.ok => c.moss,
      SStatusKind.bad => c.dangerText,
      SStatusKind.warn => c.warnRule,
      SStatusKind.info => c.accentText,
      SStatusKind.off => c.textTertiary,
    };
    return Container(
      padding: const EdgeInsets.fromLTRB(7, 0, 8, 0),
      decoration: BoxDecoration(
        color: bg,
        borderRadius: Radii.pill,
        border: kind == SStatusKind.off ? Border.all(color: c.border) : null,
      ),
      child: Row(mainAxisSize: MainAxisSize.min, children: [
        Container(width: 6, height: 6, decoration: BoxDecoration(color: dot, shape: BoxShape.circle)),
        const SizedBox(width: 5),
        Text(label, style: tsRaw(11.5, 19, 680, kind == SStatusKind.off ? c.textSecondary : c.textPrimary)),
      ]),
    );
  }
}

/// A small outlined tag ("Default", "Program", "Read and write").
class STag extends StatelessWidget {
  const STag(this.text, {super.key});
  final String text;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
      decoration: BoxDecoration(borderRadius: BorderRadius.circular(4), border: Border.all(color: c.border)),
      child: Text(text, style: tsRaw(11, 15, 600, c.textSecondary)),
    );
  }
}

/// An empty list: icon, title, a sentence and an optional action.
class SEmpty extends StatelessWidget {
  const SEmpty({super.key, required this.icon, required this.title, required this.text, this.action});
  final Sx icon;
  final String title, text;
  final Widget? action;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 20),
      decoration: BoxDecoration(color: c.bgSecondary, borderRadius: Radii.field),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        SIcon(icon, size: 22, color: c.textTertiary),
        const SizedBox(height: 8),
        Text(title, style: tsRaw(14, 19, 700, c.textPrimary)),
        const SizedBox(height: 3),
        Text(text, style: sDesc(context)),
        if (action != null) ...[const SizedBox(height: 12), action!],
      ]),
    );
  }
}

enum SNoticeKind { info, warn, bad }

/// A boxed note with a coloured left rule, and optional actions.
class SNotice extends StatelessWidget {
  const SNotice(this.text, {super.key, this.kind = SNoticeKind.info, this.actions = const [], this.textKey, this.below});
  final String text;
  final SNoticeKind kind;
  final List<Widget> actions;
  final Key? textKey;

  /// Shown under the text, full width (a token's secret to copy).
  final Widget? below;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final rule = switch (kind) {
      SNoticeKind.info => c.accent,
      SNoticeKind.warn => c.warnRule,
      SNoticeKind.bad => c.dangerText,
    };
    return Container(
      margin: const EdgeInsets.only(top: 12),
      padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
      decoration: BoxDecoration(
        color: c.bgSecondary,
        border: Border(left: BorderSide(color: rule, width: 3)),
        borderRadius: const BorderRadius.only(
          topLeft: Radius.circular(2),
          topRight: Radius.circular(8),
          bottomRight: Radius.circular(8),
          bottomLeft: Radius.circular(2),
        ),
      ),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text.rich(sRich(text, tsRaw(13.5, 19, 400, kind == SNoticeKind.bad ? c.dangerText : c.textPrimary)), key: textKey),
        if (below != null) ...[const SizedBox(height: 8), below!],
        if (actions.isNotEmpty) ...[
          const SizedBox(height: 8),
          Wrap(spacing: 8, runSpacing: 6, children: actions),
        ],
      ]),
    );
  }
}

/// The big status card at the top of a page (Sync and devices).
class SStatusCard extends StatelessWidget {
  const SStatusCard({super.key, required this.icon, required this.title, required this.text, this.ok = true, this.action});
  final Sx icon;
  final String title, text;
  final bool ok;
  final Widget? action;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      margin: const EdgeInsets.only(top: 18),
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
      decoration: BoxDecoration(color: c.bgSecondary, borderRadius: Radii.panel, border: Border.all(color: c.border)),
      child: Row(children: [
        Container(
          width: 40,
          height: 40,
          decoration: BoxDecoration(
            color: ok ? c.okBg : c.bgPrimary,
            border: ok ? null : Border.all(color: c.border),
            borderRadius: const BorderRadius.only(
              topLeft: Radius.circular(8),
              topRight: Radius.circular(12),
              bottomRight: Radius.circular(8),
              bottomLeft: Radius.circular(10),
            ),
          ),
          alignment: Alignment.center,
          child: SIcon(icon, size: 19, color: ok ? c.textPrimary : c.textTertiary),
        ),
        const SizedBox(width: 14),
        Expanded(
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Text(title, style: tsRaw(15, 20, 700, c.textPrimary)),
            const SizedBox(height: 2),
            Text(text, style: sDesc(context)),
          ]),
        ),
        if (action != null) ...[const SizedBox(width: 12), action!],
      ]),
    );
  }
}

/// A folded group of rarely needed options: chevron, title, a small note.
class SAdvanced extends StatefulWidget {
  const SAdvanced({super.key, required this.title, this.summary, required this.children, this.initiallyOpen = false, this.headerKey});
  final String title;
  final String? summary;
  final List<Widget> children;
  final bool initiallyOpen;
  final Key? headerKey;

  @override
  State<SAdvanced> createState() => _SAdvancedState();
}

class _SAdvancedState extends State<SAdvanced> {
  late bool _open = widget.initiallyOpen;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      margin: const EdgeInsets.only(top: 18),
      decoration: BoxDecoration(border: Border(top: BorderSide(color: c.border))),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Semantics(
          button: true,
          expanded: _open,
          child: Hover(
            key: widget.headerKey,
            onTap: () => setState(() => _open = !_open),
            lift: 0,
            padding: const EdgeInsets.symmetric(vertical: 12, horizontal: 2),
            child: Row(children: [
              AnimatedRotation(
                turns: _open ? .25 : 0,
                duration: Motion.of(context).fast,
                child: SIcon(Sx.chevronRight, size: 15, color: c.textTertiary),
              ),
              const SizedBox(width: 6),
              Text(widget.title, style: tsDisplay(const TypeSpec(15, 20, 620), c.textSecondary, weight: 620)),
              if (widget.summary != null) ...[
                const SizedBox(width: 8),
                Flexible(child: Text(widget.summary!, overflow: TextOverflow.ellipsis, style: sHint(context))),
              ],
            ]),
          ),
        ),
        if (_open) ..._separated(widget.children, c.border),
      ]),
    );
  }
}

/// Monospace text with a Copy button.
class SCopyLine extends StatefulWidget {
  const SCopyLine(this.text, {super.key, this.textKey, this.label});
  final String text;
  final Key? textKey;

  /// A leading label ("Claude Code") before the text.
  final String? label;

  @override
  State<SCopyLine> createState() => _SCopyLineState();
}

class _SCopyLineState extends State<SCopyLine> {
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
      constraints: const BoxConstraints(minHeight: 36),
      padding: const EdgeInsets.fromLTRB(10, 2, 2, 2),
      decoration: BoxDecoration(color: c.bgSecondary, borderRadius: Radii.field, border: Border.all(color: c.border)),
      child: Row(children: [
        if (widget.label != null) ...[
          Text(widget.label!, style: tsRaw(13, 18, 640, c.textSecondary)),
          const SizedBox(width: 10),
        ],
        Expanded(child: Text(widget.text, key: widget.textKey, style: sMono(context))),
        const SizedBox(width: 8),
        SButton(_copied ? 'Copied' : 'Copy', onTap: _copy, kind: SButtonKind.quiet),
      ]),
    );
  }
}

/// A collapsible Connectors panel: icon, title, description and a status
/// summary in the header; the body shows when open.
class SPanel extends StatelessWidget {
  const SPanel({
    super.key,
    required this.icon,
    required this.title,
    required this.desc,
    required this.status,
    required this.open,
    required this.onToggle,
    required this.child,
    this.headerKey,
    this.keepAlive = false,
  });
  final Sx icon;
  final String title, desc, status;
  final bool open;
  final VoidCallback onToggle;
  final Widget child;
  final Key? headerKey;

  /// Builds the body while closed too, offstage, so it can load what the
  /// header's status needs (the apps panel's token count).
  final bool keepAlive;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final statusText = Text(status, style: tsRaw(12.5, 17, 600, c.textSecondary));
    return Container(
      margin: const EdgeInsets.only(top: 14),
      decoration: BoxDecoration(color: c.bgPrimary, borderRadius: Radii.field, border: Border.all(color: c.border)),
      clipBehavior: Clip.antiAlias,
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Semantics(
          button: true,
          expanded: open,
          child: Hover(
            key: headerKey,
            onTap: onToggle,
            lift: 0,
            borderRadius: BorderRadius.zero,
            padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
            child: Row(crossAxisAlignment: CrossAxisAlignment.center, children: [
              Container(
                width: 32,
                height: 32,
                decoration: BoxDecoration(color: c.bgSecondary, borderRadius: BorderRadius.circular(8)),
                alignment: Alignment.center,
                child: SIcon(icon, size: 16, color: c.textSecondary),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                  Text(title, style: tsDisplay(const TypeSpec(15, 20, 620), c.textPrimary, weight: 620)),
                  const SizedBox(height: 2),
                  Text(desc, style: tsRaw(13, 18, 400, c.textTertiary)),
                  if (m) ...[const SizedBox(height: 4), statusText],
                ]),
              ),
              if (!m) ...[const SizedBox(width: 12), statusText],
              const SizedBox(width: 10),
              AnimatedRotation(
                turns: open ? .5 : 0,
                duration: Motion.of(context).fast,
                child: SIcon(Sx.chevronDown, size: 16, color: c.textTertiary),
              ),
            ]),
          ),
        ),
        if (open || keepAlive)
          Offstage(
            offstage: !open,
            child: Container(
              padding: const EdgeInsets.fromLTRB(16, 4, 16, 16),
              decoration: BoxDecoration(border: Border(top: BorderSide(color: c.border))),
              child: child,
            ),
          ),
      ]),
    );
  }
}

/// Which panels on Settings > Connectors are open ('tools', 'apps', 'hub').
/// They start closed; other pages open one before jumping there
/// ([showMcpServers]), and the Connectors page clears this when it closes.
final connectorPanels = ValueNotifier<Set<String>>(<String>{});

// AppTooltip: the one tooltip for icon-only buttons. A small elevated label
// with the shortcut as a muted key hint, taken from the command registry
// when the button runs a command, so the two never drift.
//
// Desktop: shows after [AppTooltip.hoverDelay] of hover and on keyboard
// focus; hides on pointer leave, a click, Escape or scroll.
// Touch: shows on long-press (which then does not reach the button's tap)
// and hides shortly after release.

import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../registry/command_registry.dart' show commandRegistry;
import 'tokens.dart';

/// "Ctrl+Shift+F" for [a].
String shortcutLabel(SingleActivator a) {
  final key = a.trigger.keyLabel;
  return [
    if (a.control) 'Ctrl',
    if (a.meta) 'Cmd',
    if (a.alt) 'Alt',
    if (a.shift) 'Shift',
    key.length == 1 ? key.toUpperCase() : key,
  ].join('+');
}

/// The first shortcut of command [id] as text, or null.
String? commandShortcutLabel(String id) {
  final s = commandRegistry.of(id)?.shortcuts;
  return s == null || s.isEmpty ? null : shortcutLabel(s.first);
}

class AppTooltip extends StatefulWidget {
  const AppTooltip({super.key, required this.label, this.shortcut, this.command, required this.child});

  /// The text; use the button's semantic label.
  final String label;

  /// A shortcut hint for buttons that do not run a registered command.
  final String? shortcut;

  /// The command the button runs; its first shortcut becomes the hint.
  final String? command;
  final Widget child;

  static const hoverDelay = Duration(milliseconds: 500);

  /// How long a touch tooltip stays after the finger lifts.
  static const touchLinger = Duration(milliseconds: 1500);

  @override
  State<AppTooltip> createState() => _AppTooltipState();
}

class _AppTooltipState extends State<AppTooltip> with SingleTickerProviderStateMixin {
  static _AppTooltipState? _shown;
  static DateTime? _lastHidden;

  final _portal = OverlayPortalController();
  late final AnimationController _fade = AnimationController(vsync: this);
  Timer? _timer;
  ScrollPosition? _scroll;
  bool _keyHandler = false;

  String? get _shortcut =>
      widget.shortcut ?? (widget.command == null ? null : commandShortcutLabel(widget.command!));

  void _schedule(Duration delay) {
    _timer?.cancel();
    // Moving straight from one tooltip to the next skips the delay.
    final warm = _lastHidden != null && DateTime.now().difference(_lastHidden!) < const Duration(milliseconds: 300);
    if (delay == Duration.zero || warm) {
      _show();
    } else {
      _timer = Timer(delay, _show);
    }
  }

  void _show() {
    _timer?.cancel();
    if (!mounted || _portal.isShowing || Overlay.maybeOf(context) == null) return;
    if (_shown != this) _shown?._hide();
    _shown = this;
    _fade.duration = Motion.of(context).fast;
    _portal.show();
    _fade.forward(from: 0);
    _scroll = Scrollable.maybeOf(context)?.position;
    _scroll?.addListener(_hide);
    if (!_keyHandler) {
      HardwareKeyboard.instance.addHandler(_onKey);
      _keyHandler = true;
    }
  }

  void _hide() => _stop(disposing: false);

  void _stop({required bool disposing}) {
    _timer?.cancel();
    _timer = null;
    _scroll?.removeListener(_hide);
    _scroll = null;
    if (_keyHandler) {
      HardwareKeyboard.instance.removeHandler(_onKey);
      _keyHandler = false;
    }
    if (_shown == this) _shown = null;
    if (_portal.isShowing) {
      _lastHidden = DateTime.now();
      _fade.stop();
      if (!disposing) _portal.hide();
    }
  }

  bool _onKey(KeyEvent e) {
    if (e is KeyDownEvent && e.logicalKey == LogicalKeyboardKey.escape) _hide();
    return false;
  }

  void _onFocus(bool focused) {
    if (focused && FocusManager.instance.highlightMode == FocusHighlightMode.traditional) {
      _schedule(Duration.zero);
    } else if (!focused) {
      _hide();
    }
  }

  @override
  void dispose() {
    _stop(disposing: true);
    _fade.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return OverlayPortal(
      controller: _portal,
      overlayChildBuilder: _bubble,
      child: Focus(
        canRequestFocus: false,
        skipTraversal: true,
        onFocusChange: _onFocus,
        child: MouseRegion(
          onEnter: (e) {
            if (e.kind == PointerDeviceKind.mouse) _schedule(AppTooltip.hoverDelay);
          },
          onExit: (_) => _hide(),
          child: Listener(
            onPointerDown: (e) {
              if (e.kind == PointerDeviceKind.mouse) _hide();
            },
            child: RawGestureDetector(
              gestures: {
                LongPressGestureRecognizer: GestureRecognizerFactoryWithHandlers<LongPressGestureRecognizer>(
                  () => LongPressGestureRecognizer(
                    debugOwner: this,
                    supportedDevices: const {PointerDeviceKind.touch, PointerDeviceKind.stylus},
                  ),
                  (r) => r
                    ..onLongPress = () {
                      HapticFeedback.selectionClick();
                      _show();
                    }
                    ..onLongPressEnd = (_) {
                      _timer?.cancel();
                      _timer = Timer(AppTooltip.touchLinger, _hide);
                    },
                ),
              },
              child: widget.child,
            ),
          ),
        ),
      ),
    );
  }

  Widget _bubble(BuildContext context) {
    final box = this.context.findRenderObject() as RenderBox?;
    final overlay = Overlay.maybeOf(this.context)?.context.findRenderObject() as RenderBox?;
    if (box == null || !box.hasSize || overlay == null) return const SizedBox.shrink();
    final anchor = box.localToGlobal(Offset.zero, ancestor: overlay) & box.size;
    final c = context.colors;
    final type = context.type;
    final shortcut = _shortcut;
    return Positioned.fill(
      child: IgnorePointer(
        child: ExcludeSemantics(
          child: CustomSingleChildLayout(
            delegate: _Below(anchor, MediaQuery.paddingOf(context)),
            child: FadeTransition(
              opacity: CurvedAnimation(parent: _fade, curve: Motion.ease),
              child: Container(
                key: const ValueKey('app-tooltip'),
                constraints: const BoxConstraints(maxWidth: 280),
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 5),
                decoration: BoxDecoration(
                  color: c.bgElevated,
                  borderRadius: BorderRadius.circular(Radii.md),
                  border: Border.all(color: c.borderSubtle),
                  boxShadow: c.seamHover,
                ),
                child: Text.rich(
                  TextSpan(children: [
                    TextSpan(text: widget.label, style: ts(type.uiSmall, c.textPrimary)),
                    if (shortcut != null)
                      TextSpan(text: '  $shortcut', style: ts(type.uiSmall, c.textTertiary, weight: 400)),
                  ]),
                  textScaler: MediaQuery.textScalerOf(context).clamp(maxScaleFactor: 1.5),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// Below the anchor (above when there is no room), centred on it and kept
/// [_margin] inside the window and its safe area.
class _Below extends SingleChildLayoutDelegate {
  _Below(this.anchor, this.safe);
  final Rect anchor;
  final EdgeInsets safe;
  static const _gap = 6.0, _margin = 8.0;

  @override
  BoxConstraints getConstraintsForChild(BoxConstraints c) =>
      c.loosen().deflate(EdgeInsets.all(_margin) + safe);

  @override
  Offset getPositionForChild(Size size, Size child) {
    final left = safe.left + _margin, right = size.width - safe.right - _margin - child.width;
    final x = (anchor.center.dx - child.width / 2).clamp(left, right < left ? left : right);
    final below = anchor.bottom + _gap;
    final fitsBelow = below + child.height <= size.height - safe.bottom - _margin;
    final above = anchor.top - _gap - child.height;
    final y = fitsBelow || above < safe.top + _margin ? below : above;
    final top = safe.top + _margin, bottom = size.height - safe.bottom - _margin - child.height;
    return Offset(x, y.clamp(top, bottom < top ? top : bottom));
  }

  @override
  bool shouldRelayout(_Below old) => old.anchor != anchor || old.safe != safe;
}

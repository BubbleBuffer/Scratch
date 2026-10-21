// The conflict toast: a short line with an optional Undo, shown over the
// page for six seconds. Copied from addons/tasks/calendar_drop_toast.dart
// (not imported, so features/sync does not depend on an add-on). A new toast
// replaces the one showing. The timer belongs to the toast's own state, so it
// stops when the toast leaves, including when the overlay closes.

import 'dart:async';

import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'conflict_copy.dart';

/// The toast on screen, if any, and how to hide it.
OverlayEntry? _shownEntry;
VoidCallback? _hideShown;

/// Shows [message] over [overlay]. Returns an idempotent dismiss callback.
/// Take [overlay] before any await, and check [OverlayState.mounted] before
/// calling this: the calling widget and the overlay may be gone by then.
/// [area] is how far the page sits from the overlay's left and right edges;
/// the toast centres on the page, not the window.
VoidCallback showConflictToast(
  OverlayState overlay,
  String message, {
  EdgeInsets area = EdgeInsets.zero,
  Future<void> Function()? undo,
  required void Function(Object) onError,
}) {
  _hideShown?.call();
  late final OverlayEntry entry;
  var hidden = false;
  void forget() {
    if (!identical(_shownEntry, entry)) return;
    _shownEntry = null;
    _hideShown = null;
  }

  void hide() {
    if (hidden) return;
    hidden = true;
    forget();
    // A closed overlay has already dropped its entries; removing one again fails.
    if (overlay.mounted) entry.remove();
  }

  entry = OverlayEntry(
    builder: (_) => _ConflictToast(
      message: message,
      area: area,
      undo: undo,
      onError: onError,
      hide: hide,
      forget: forget,
    ),
  );
  overlay.insert(entry);
  _shownEntry = entry;
  _hideShown = hide;
  return hide;
}

class _ConflictToast extends StatefulWidget {
  const _ConflictToast({
    required this.message,
    required this.area,
    required this.undo,
    required this.onError,
    required this.hide,
    required this.forget,
  });
  final String message;
  final EdgeInsets area;
  final Future<void> Function()? undo;
  final void Function(Object) onError;

  /// Removes the toast (idempotent).
  final VoidCallback hide;

  /// Clears the module's record of this toast; called when its state goes.
  final VoidCallback forget;

  @override
  State<_ConflictToast> createState() => _ConflictToastState();
}

class _ConflictToastState extends State<_ConflictToast> {
  late Timer _timer;

  @override
  void initState() {
    super.initState();
    _timer = Timer(const Duration(seconds: 6), widget.hide);
  }

  @override
  void dispose() {
    _timer.cancel();
    widget.forget();
    super.dispose();
  }

  Future<void> _undo() async {
    final undo = widget.undo;
    final onError = widget.onError;
    widget.hide();
    if (undo == null) return;
    try {
      await undo();
    } catch (error) {
      onError(error);
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final undo = widget.undo;
    return Positioned(
      left: widget.area.left + (m ? 16 : 24),
      right: widget.area.right + (m ? 16 : 24),
      bottom: MediaQuery.viewInsetsOf(context).bottom + 24,
      child: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 420),
          child: Container(
            key: const ValueKey('conflict-toast'),
            padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
            decoration: BoxDecoration(
              color: c.rail,
              borderRadius: BorderRadius.circular(10),
              boxShadow: const [BoxShadow(color: Color(0x47141618), blurRadius: 20, offset: Offset(0, 8))],
            ),
            child: Row(children: [
              Expanded(child: Text(widget.message, style: tsRaw(14, 20, 400, c.railInk))),
              if (undo != null) ...[
                const SizedBox(width: 16),
                Semantics(
                  button: true,
                  label: conflictUndo,
                  child: Hover(
                    key: const ValueKey('conflict-toast-undo'),
                    onTap: _undo,
                    radius: Radii.sm,
                    padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
                    // The dark theme accent: the rail is dark in both themes.
                    child: Text(conflictUndo, style: tsRaw(14, 20, 700, AppColors.darkTheme.accent)),
                  ),
                ),
              ],
            ]),
          ),
        ),
      ),
    );
  }
}

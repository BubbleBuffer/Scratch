// The "Marked done" message with Undo. Same structure as the calendar drop
// toast in the tasks add-on; that file is not exported, so the pattern is
// repeated here rather than imported.

import 'dart:async';

import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';

/// Shows [message] over the bottom of the screen, above the phone's footer.
/// Returns an idempotent dismiss callback.
VoidCallback showQueryToast(
  BuildContext context,
  String message, {
  Future<void> Function()? undo,
  void Function(Object error)? onError,
  VoidCallback? onDismissed,
}) {
  late final OverlayEntry entry;
  Timer? timer;
  var dismissed = false;
  void dismiss() {
    if (dismissed) return;
    dismissed = true;
    timer?.cancel();
    entry.remove();
    onDismissed?.call();
  }

  entry = OverlayEntry(
    builder: (ctx) => Positioned(
      left: 24,
      right: 24,
      bottom: MediaQuery.viewInsetsOf(ctx).bottom + 80,
      child: Align(
        alignment: Alignment.bottomCenter,
        child: MenuSurface(
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
            child: Row(
              key: const ValueKey('query-toast'),
              mainAxisSize: MainAxisSize.min,
              children: [
                Flexible(
                  child: Text(
                    message,
                    style: ts(ctx.type.ui, ctx.colors.textPrimary),
                  ),
                ),
                if (undo != null) ...[
                  const SizedBox(width: 16),
                  TextBtn(
                    'Undo',
                    key: const ValueKey('query-toast-undo'),
                    onTap: () async {
                      dismiss();
                      try {
                        await undo();
                      } catch (error) {
                        onError?.call(error);
                      }
                    },
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    ),
  );
  Overlay.of(context).insert(entry);
  timer = Timer(const Duration(seconds: 6), dismiss);
  return dismiss;
}

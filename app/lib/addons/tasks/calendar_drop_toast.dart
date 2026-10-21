import 'dart:async';

import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';

/// Returns an idempotent dismiss callback so the screen owns its overlay.
VoidCallback showCalendarDropToast(
  BuildContext context,
  String message, {
  Future<void> Function()? undo,
  required void Function(Object) onError,
}) {
  late final OverlayEntry entry;
  Timer? timer;
  var dismissed = false;
  void dismiss() {
    if (dismissed) return;
    dismissed = true;
    timer?.cancel();
    entry.remove();
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
              key: const ValueKey('calendar-drop-toast'),
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
                    onTap: () async {
                      dismiss();
                      try {
                        await undo();
                      } catch (error) {
                        onError(error);
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

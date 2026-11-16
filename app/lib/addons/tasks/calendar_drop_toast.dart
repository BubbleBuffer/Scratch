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
      left: Space.xl,
      right: Space.xl,
      bottom: MediaQuery.viewInsetsOf(ctx).bottom + ToastPlacement.bottom,
      child: Align(
        alignment: Alignment.bottomCenter,
        child: MenuSurface(
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: Space.lg, vertical: Space.md),
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
                  const Gap(Space.lg),
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

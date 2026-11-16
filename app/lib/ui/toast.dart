import 'dart:async';

import 'package:flutter/widgets.dart';

import 'tokens.dart';

/// Brief, non-blocking feedback on the app's existing elevated surface.
void showErrorToast(BuildContext context, String message) =>
    _showToast(context, message, 'error-toast');

/// A brief, non-blocking confirmation, on the same surface as the errors.
void showToast(BuildContext context, String message) =>
    _showToast(context, message, 'toast');

void _showToast(BuildContext context, String message, String name) {
  final overlay = Overlay.of(context);
  late final OverlayEntry entry;
  entry = OverlayEntry(
    builder: (_) =>
        _Toast(message: message, name: name, remove: entry.remove),
  );
  overlay.insert(entry);
}

class _Toast extends StatefulWidget {
  const _Toast({
    required this.message,
    required this.name,
    required this.remove,
  });
  final String message;
  final String name;
  final VoidCallback remove;
  @override
  State<_Toast> createState() => _ToastState();
}

class _ToastState extends State<_Toast> {
  late final Timer _timer = Timer(Motion.toast, widget.remove);
  @override
  void initState() {
    super.initState();
    _timer;
  }

  @override
  void dispose() {
    _timer.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Positioned(
      left: Space.xl,
      right: Space.xl,
      bottom: MediaQuery.viewInsetsOf(context).bottom + ToastPlacement.bottom,
      child: IgnorePointer(
        child: Align(
          alignment: Alignment.bottomCenter,
          child: Container(
            key: ValueKey(widget.name),
            padding: const EdgeInsets.symmetric(horizontal: Space.lg, vertical: Space.md),
            decoration: BoxDecoration(
              color: c.textPrimary,
              borderRadius: Radii.panel,
              boxShadow: c.seamHover,
            ),
            child: Text(
              widget.message,
              style: ts(context.type.ui, c.bgPrimary),
            ),
          ),
        ),
      ),
    );
  }
}

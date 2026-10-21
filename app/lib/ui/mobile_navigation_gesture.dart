// Mobile navigation participates in the same arena as the content below it.
// Inner horizontal scrollers/drag handlers enter first and keep their gestures.
import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';

class MobileNavigationGesture extends StatelessWidget {
  const MobileNavigationGesture({
    super.key,
    required this.child,
    this.enabled = true,
    this.canStart,
    this.onStart,
    required this.onUpdate,
    required this.onEnd,
    this.onCancel,
  });

  final Widget child;
  final bool enabled;
  final bool Function()? canStart;
  final VoidCallback? onStart, onCancel;
  final GestureDragUpdateCallback onUpdate;
  final GestureDragEndCallback onEnd;

  @override
  Widget build(BuildContext context) {
    final route = ModalRoute.of(context);
    bool allowed() {
      if (!enabled || route?.isCurrent == false || canStart?.call() == false) {
        return false;
      }
      // This covers capture, the chat composer and editor text alike. Read at
      // pointer-down, since selecting text need not rebuild this ancestor.
      final focus = FocusManager.instance.primaryFocus?.context;
      final editable = focus?.findAncestorStateOfType<EditableTextState>();
      // Flutter exposes handle visibility through this public state getter,
      // annotated for tests. It is also needed here for collapsed caret handles.
      // ignore: invalid_use_of_visible_for_testing_member
      if (editable?.selectionOverlay?.handlesAreVisible == true) return false;
      final selection = editable?.widget.controller.selection;
      return selection == null || !selection.isValid || selection.isCollapsed;
    }

    return RawGestureDetector(
      behavior: HitTestBehavior.opaque,
      gestures: {
        _NavigationDrag: GestureRecognizerFactoryWithHandlers<_NavigationDrag>(
          () => _NavigationDrag(debugOwner: this),
          (recognizer) {
            recognizer
              ..canStart = allowed
              ..onStart = (_) {
                recognizer.navigationStarted = true;
                onStart?.call();
              }
              ..onUpdate = onUpdate
              ..onEnd = (details) {
                recognizer.navigationStarted = false;
                onEnd(details);
              }
              ..onCancel = () {
                // Losing the arena also calls onCancel, without onStart.
                // Only a navigation drag that actually began needs settling.
                if (!recognizer.navigationStarted) return;
                recognizer.navigationStarted = false;
                onCancel?.call();
              };
          },
        ),
      },
      child: child,
    );
  }
}

class _NavigationDrag extends HorizontalDragGestureRecognizer {
  _NavigationDrag({super.debugOwner}) {
    // Being the sole arena member is not sufficient: taps/vertical drags in
    // otherwise empty areas must still cross our horizontal threshold.
    onlyAcceptDragOnThreshold = true;
    // After acceptance, include the movement that crossed the threshold so
    // the sheet follows the finger even with a single large pointer update.
    dragStartBehavior = DragStartBehavior.down;
  }
  bool Function()? canStart;
  bool navigationStarted = false;
  final Map<int, Offset> _movement = {};

  @override
  bool isPointerAllowed(PointerEvent event) =>
      canStart?.call() != false && super.isPointerAllowed(event);

  @override
  void addAllowedPointer(PointerDownEvent event) {
    _movement[event.pointer] = Offset.zero;
    super.addAllowedPointer(event);
  }

  @override
  void handleEvent(PointerEvent event) {
    if (event is PointerMoveEvent && _movement.containsKey(event.pointer)) {
      _movement[event.pointer] = _movement[event.pointer]! + event.delta;
    }
    super.handleEvent(event);
    if (event is PointerUpEvent || event is PointerCancelEvent) {
      _movement.remove(event.pointer);
    }
  }

  @override
  bool hasSufficientGlobalDistanceToAccept(
    PointerDeviceKind pointerDeviceKind,
    double? deviceTouchSlop,
  ) {
    final movement = _movement.values.fold(Offset.zero, (a, b) => a + b);
    return movement.dx.abs() > movement.dy.abs() &&
        super.hasSufficientGlobalDistanceToAccept(
          pointerDeviceKind,
          deviceTouchSlop,
        );
  }

  @override
  void rejectGesture(int pointer) {
    super.rejectGesture(pointer);
    _movement.remove(pointer);
  }
}

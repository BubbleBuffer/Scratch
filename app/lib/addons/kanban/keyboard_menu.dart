// Keyboard control for a popover menu: focus starts on the first item, the
// arrow keys move between items, Esc closes the menu and gives focus back to
// the control that opened it. Items are the kit's MenuItem rows, which take
// focus and activate on Enter or Space.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

class KeyboardMenu extends StatefulWidget {
  const KeyboardMenu({
    super.key,
    required this.child,
    this.restoreFocus,
  });

  final Widget child;

  /// The node that takes focus when Esc closes the menu.
  final FocusNode? restoreFocus;

  @override
  State<KeyboardMenu> createState() => _KeyboardMenuState();
}

class _KeyboardMenuState extends State<KeyboardMenu> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) FocusScope.of(context).nextFocus();
    });
  }

  void _close() {
    Navigator.of(context).maybePop();
    widget.restoreFocus?.requestFocus();
  }

  @override
  Widget build(BuildContext context) {
    return FocusTraversalGroup(
      child: Shortcuts(
        shortcuts: const {
          SingleActivator(LogicalKeyboardKey.arrowDown): NextFocusIntent(),
          SingleActivator(LogicalKeyboardKey.arrowUp): PreviousFocusIntent(),
          SingleActivator(LogicalKeyboardKey.escape): DismissIntent(),
        },
        child: Actions(
          actions: {
            NextFocusIntent: CallbackAction<NextFocusIntent>(
              onInvoke: (_) => FocusScope.of(context).nextFocus(),
            ),
            PreviousFocusIntent: CallbackAction<PreviousFocusIntent>(
              onInvoke: (_) => FocusScope.of(context).previousFocus(),
            ),
            DismissIntent: CallbackAction<DismissIntent>(
              onInvoke: (_) {
                _close();
                return null;
              },
            ),
          },
          child: widget.child,
        ),
      ),
    );
  }
}

// The `key` command: a key combination delivered the way the engine delivers
// a physical key, as key events to the focus tree. The keyboard
// state, focus handlers and app shortcuts see an ordinary key press.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'remote_handler.dart';

class _Key {
  const _Key(this.physical, this.logical, [this.character]);
  final PhysicalKeyboardKey physical;
  final LogicalKeyboardKey logical;
  final String? character;
}

const _modifierKeys = {
  'ctrl': _Key(PhysicalKeyboardKey.controlLeft, LogicalKeyboardKey.controlLeft),
  'shift': _Key(PhysicalKeyboardKey.shiftLeft, LogicalKeyboardKey.shiftLeft),
  'alt': _Key(PhysicalKeyboardKey.altLeft, LogicalKeyboardKey.altLeft),
  'meta': _Key(PhysicalKeyboardKey.metaLeft, LogicalKeyboardKey.metaLeft),
};

const _named = {
  'enter': _Key(PhysicalKeyboardKey.enter, LogicalKeyboardKey.enter),
  'escape': _Key(PhysicalKeyboardKey.escape, LogicalKeyboardKey.escape),
  'tab': _Key(PhysicalKeyboardKey.tab, LogicalKeyboardKey.tab),
  'up': _Key(PhysicalKeyboardKey.arrowUp, LogicalKeyboardKey.arrowUp),
  'down': _Key(PhysicalKeyboardKey.arrowDown, LogicalKeyboardKey.arrowDown),
  'left': _Key(PhysicalKeyboardKey.arrowLeft, LogicalKeyboardKey.arrowLeft),
  'right': _Key(PhysicalKeyboardKey.arrowRight, LogicalKeyboardKey.arrowRight),
  'backspace': _Key(
    PhysicalKeyboardKey.backspace,
    LogicalKeyboardKey.backspace,
  ),
  'delete': _Key(PhysicalKeyboardKey.delete, LogicalKeyboardKey.delete),
  'space': _Key(PhysicalKeyboardKey.space, LogicalKeyboardKey.space, ' '),
  'f1': _Key(PhysicalKeyboardKey.f1, LogicalKeyboardKey.f1),
  'f2': _Key(PhysicalKeyboardKey.f2, LogicalKeyboardKey.f2),
  'f3': _Key(PhysicalKeyboardKey.f3, LogicalKeyboardKey.f3),
  'f4': _Key(PhysicalKeyboardKey.f4, LogicalKeyboardKey.f4),
  'f5': _Key(PhysicalKeyboardKey.f5, LogicalKeyboardKey.f5),
  'f6': _Key(PhysicalKeyboardKey.f6, LogicalKeyboardKey.f6),
  'f7': _Key(PhysicalKeyboardKey.f7, LogicalKeyboardKey.f7),
  'f8': _Key(PhysicalKeyboardKey.f8, LogicalKeyboardKey.f8),
  'f9': _Key(PhysicalKeyboardKey.f9, LogicalKeyboardKey.f9),
  'f10': _Key(PhysicalKeyboardKey.f10, LogicalKeyboardKey.f10),
  'f11': _Key(PhysicalKeyboardKey.f11, LogicalKeyboardKey.f11),
  'f12': _Key(PhysicalKeyboardKey.f12, LogicalKeyboardKey.f12),
};

// Letters and digits: HID usages and logical ids are consecutive from a / 1.
// The logical id of a letter is its lowercase code point.
_Key _alnum(String name, {required bool printable, required bool shift}) {
  final c = name.codeUnitAt(0);
  if (c >= 0x61 && c <= 0x7a) {
    final char = printable ? (shift ? name.toUpperCase() : name) : null;
    return _Key(
      PhysicalKeyboardKey(PhysicalKeyboardKey.keyA.usbHidUsage + (c - 0x61)),
      LogicalKeyboardKey(c),
      char,
    );
  }
  final physical = c == 0x30
      ? PhysicalKeyboardKey.digit0
      : PhysicalKeyboardKey(
          PhysicalKeyboardKey.digit1.usbHidUsage + (c - 0x31),
        );
  return _Key(physical, LogicalKeyboardKey(c), printable ? name : null);
}

_Key _resolve(KeyCombo combo) {
  final named = _named[combo.key];
  if (named != null) return named;
  // Control, alt or meta make a shortcut, not a typed character.
  final printable = !combo.modifiers.any((m) => m != 'shift');
  return _alnum(
    combo.key,
    printable: printable,
    shift: combo.modifiers.contains('shift'),
  );
}

/// Presses and releases [combo]: modifiers down, the key down and up,
/// modifiers up. Throws a 500 [RemoteError] when the app is not ready.
void pressKeyCombo(KeyCombo combo) {
  // ignore: deprecated_member_use
  final manager = ServicesBinding.instance.keyEventManager;
  final key = _resolve(combo);
  final mods = [
    for (final m in keyModifiers)
      if (combo.modifiers.contains(m)) _modifierKeys[m]!,
  ];
  var stamp = Duration(microseconds: DateTime.now().microsecondsSinceEpoch);

  void send(bool down, _Key k, {String? character}) {
    stamp += const Duration(microseconds: 1);
    final KeyEvent event = down
        ? KeyDownEvent(
            physicalKey: k.physical,
            logicalKey: k.logical,
            character: character,
            timeStamp: stamp,
          )
        : KeyUpEvent(
            physicalKey: k.physical,
            logicalKey: k.logical,
            timeStamp: stamp,
          );
    // The engine's key-data path pairs events with legacy raw messages that
    // never arrive here, so update the keyboard state and hand the event to
    // the focus tree directly.
    HardwareKeyboard.instance.handleKeyEvent(event);
    // ignore: deprecated_member_use
    manager.keyMessageHandler?.call(
      // ignore: deprecated_member_use
      KeyMessage(<KeyEvent>[event], null),
    );
  }

  for (final m in mods) {
    send(true, m);
  }
  try {
    send(true, key, character: key.character);
    send(false, key);
    if (combo.key == 'enter' && combo.modifiers.isEmpty) submitFocusedTextField();
  } finally {
    for (final m in mods.reversed) {
      send(false, m);
    }
  }
}

/// On a desktop keyboard Enter reaches a text field twice: as a key event and
/// as the platform's text-input action, which is what submits the field. Key
/// events alone never produce the action, so a remote Enter sends it to the
/// focused field itself, unless the field is multiline (Enter is a newline).
void submitFocusedTextField() {
  final context = FocusManager.instance.primaryFocus?.context;
  final field = context?.findAncestorStateOfType<EditableTextState>();
  if (field == null) return;
  final widget = field.widget;
  if (widget.maxLines != 1) return;
  field.performAction(widget.textInputAction ?? TextInputAction.done);
}

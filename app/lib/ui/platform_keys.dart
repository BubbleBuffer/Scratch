// Shortcut keys that follow the platform: the command key is Cmd on macOS and
// iOS, Ctrl elsewhere. Commands declare Ctrl shortcuts; these helpers turn a
// declared Ctrl into Cmd for binding and for the text shown next to a command.

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'tooltip.dart' as bb;

/// Whether the command key is Cmd (macOS, iOS) rather than Ctrl.
bool get usesCmdKey =>
    defaultTargetPlatform == TargetPlatform.macOS ||
    defaultTargetPlatform == TargetPlatform.iOS;

/// [a] as the platform binds it: a declared Ctrl becomes Cmd on macOS.
SingleActivator platformActivator(SingleActivator a) => !usesCmdKey || !a.control
    ? a
    : SingleActivator(
        a.trigger,
        meta: true,
        alt: a.alt,
        shift: a.shift,
        includeRepeats: a.includeRepeats,
      );

/// Whether the command key is down (Cmd on macOS, Ctrl elsewhere).
bool get isCommandKeyPressed => usesCmdKey
    ? HardwareKeyboard.instance.isMetaPressed
    : HardwareKeyboard.instance.isControlPressed;

/// The text for [a]: "Ctrl+Shift+F", or "⌘⇧F" on macOS.
String shortcutText(SingleActivator declared) {
  if (!usesCmdKey) return bb.shortcutLabel(declared);
  final a = platformActivator(declared);
  final key = a.trigger.keyLabel;
  return [
    if (a.control) '⌃',
    if (a.alt) '⌥',
    if (a.shift) '⇧',
    if (a.meta) '⌘',
    key.length == 1 ? key.toUpperCase() : key,
  ].join();
}

/// A fixed hint written with Ctrl ("Ctrl+Shift+F"), as the platform shows it.
String shortcutHint(String ctrlHint) {
  if (!usesCmdKey || !ctrlHint.startsWith('Ctrl')) return ctrlHint;
  final parts = ctrlHint.split(RegExp(r'[+ ]')).skip(1).toList();
  final key = parts.removeLast();
  return [
    if (parts.contains('Alt')) '⌥',
    if (parts.contains('Shift')) '⇧',
    '⌘',
    key,
  ].join();
}

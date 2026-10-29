// Platform keys: a declared Ctrl binds and reads as Cmd on macOS, stays Ctrl
// elsewhere.

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/ui/ui.dart';

const _ctrlShiftF = SingleActivator(
  LogicalKeyboardKey.keyF,
  control: true,
  shift: true,
);

void main() {
  test('Ctrl stays Ctrl off macOS', () {
    for (final p in [TargetPlatform.windows, TargetPlatform.linux, TargetPlatform.android]) {
      debugDefaultTargetPlatformOverride = p;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      final bound = platformActivator(_ctrlShiftF);
      expect(bound.control, isTrue);
      expect(bound.meta, isFalse);
      expect(shortcutText(_ctrlShiftF), 'Ctrl+Shift+F');
      expect(shortcutHint('Ctrl+Shift+F'), 'Ctrl+Shift+F');
      expect(shortcutHint('Ctrl K'), 'Ctrl K');
    }
  });

  test('Ctrl becomes Cmd on macOS', () {
    debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final bound = platformActivator(_ctrlShiftF);
    expect(bound.control, isFalse);
    expect(bound.meta, isTrue);
    expect(bound.shift, isTrue);
    expect(bound.trigger, LogicalKeyboardKey.keyF);
    expect(shortcutText(_ctrlShiftF), '⌘⇧F');
    expect(shortcutText(platformActivator(_ctrlShiftF)), '⌘⇧F');
    expect(shortcutHint('Ctrl+Shift+F'), '⇧⌘F');
    expect(shortcutHint('Ctrl+J'), '⌘J');
    expect(shortcutHint('Ctrl K'), '⌘K');
    expect(shortcutHint('Esc'), 'Esc');
  });
}

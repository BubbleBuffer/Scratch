import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/registry/addon_registry.dart';
import 'package:notes_app/shell/addons.dart';

void main() {
  test('Tasks boots in isolation and owns task presentations and commands', () {
    final addon = addonList
        .singleWhere((entry) => entry.addon.id == 'tasks')
        .addon;
    final context = AddonContext();
    addTearDown(context.noteTypes.dispose);
    bootAddons([addon], context);
    final type = context.noteTypes.of('task');
    expect(type, isNotNull);
    expect(type!.chip, isNotNull);
    expect(type.card, isNotNull);
    expect(type.embed, isNotNull);
    expect(type.fullPage, isNotNull);
    expect(
      context.commands.all.any((command) => command.label == 'New task'),
      isTrue,
    );
    final calendar = context.commands.of('calendar.open');
    expect(calendar, isNotNull);
    expect(
      calendar!.shortcuts.any(
        (shortcut) =>
            shortcut.trigger == LogicalKeyboardKey.keyC &&
            shortcut.control &&
            shortcut.shift,
      ),
      isTrue,
    );
    expect(context.commands.of('page.new'), isNull);
    expect(
      context.commands.all
          .expand((command) => command.shortcuts)
          .any(
            (shortcut) =>
                shortcut.trigger == LogicalKeyboardKey.keyN && shortcut.control,
          ),
      isFalse,
    );
  });
}

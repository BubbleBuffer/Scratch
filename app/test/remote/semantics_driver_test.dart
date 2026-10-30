// The real semantics-tree implementation of tree, tap, type and key, on a
// small widget tree.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/remote/key_press.dart';
import 'package:notes_app/remote/remote_handler.dart';
import 'package:notes_app/remote/semantics_driver.dart';

void main() {
  testWidgets('tree lists the button and the field; tap and type act', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    addTearDown(semantics.dispose);
    final controller = TextEditingController();
    addTearDown(controller.dispose);
    var taps = 0;
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Column(
            children: [
              Semantics(
                container: true,
                identifier: 'save-button',
                button: true,
                label: 'Save',
                onTap: () => taps++,
                child: const SizedBox(width: 120, height: 40),
              ),
              SizedBox(
                width: 300,
                child: Semantics(
                  container: true,
                  identifier: field.id,
                  child: TextField(
                    controller: controller,
                    decoration: const InputDecoration(labelText: 'Title'),
                  ),
                ),
              ),
              const Text('Plain words'),
            ],
          ),
        ),
      ),
    );

    final driver = SemanticsDriver(nextFrame: () async => tester.pump());

    final nodes = driver.snapshot();
    final byId = {for (final n in nodes) n.id: n};
    expect(byId.keys, contains('save-button'));
    final field = nodes.singleWhere((n) => n.role == 'textfield');
    final save = byId['save-button']!;
    expect(save.role, 'button');
    expect(save.label, 'Save');
    expect(save.actions, contains('tap'));
    expect(save.enabled, isTrue);
    final json = save.toJson();
    expect((json['rect']! as Map)['w'], 120.0);
    // Reading order: top to bottom.
    final tops = [for (final n in nodes) n.rect.top];
    expect(tops, orderedEquals([...tops]..sort()));

    driver.tap('save-button');
    await tester.pump();
    expect(taps, 1);

    await driver.type(field.id, 'Hello', replace: true);
    await tester.pump();
    expect(controller.text, 'Hello');
    await driver.type(field.id, ' world', replace: false);
    await tester.pump();
    expect(controller.text, 'Hello world');
    await driver.type(field.id, 'Fresh', replace: true);
    await tester.pump();
    expect(controller.text, 'Fresh');
  });

  testWidgets('unknown ids are 404; wrong targets are 400', (tester) async {
    final semantics = tester.ensureSemantics();
    addTearDown(semantics.dispose);
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Semantics(
            container: true,
            identifier: 'plain',
            label: 'Just a label',
            child: const SizedBox(width: 100, height: 40),
          ),
        ),
      ),
    );
    final driver = SemanticsDriver(nextFrame: () async => tester.pump());
    expect(
      () => driver.tap('missing'),
      throwsA(isA<RemoteError>().having((e) => e.status, 'status', 404)),
    );
    expect(
      () => driver.tap('plain'),
      throwsA(isA<RemoteError>().having((e) => e.status, 'status', 400)),
    );
    await expectLater(
      driver.type('plain', 'x', replace: true),
      throwsA(isA<RemoteError>().having((e) => e.status, 'status', 400)),
    );
  });

  testWidgets('key reaches a focused widget as a key event', (tester) async {
    final seen = <String>[];
    await tester.pumpWidget(
      MaterialApp(
        home: Focus(
          autofocus: true,
          onKeyEvent: (node, event) {
            if (event is KeyDownEvent) {
              final ctrl = HardwareKeyboard.instance.isControlPressed;
              if (event.logicalKey == LogicalKeyboardKey.keyK) {
                seen.add(ctrl ? 'ctrl+k' : 'k');
              } else if (event.logicalKey == LogicalKeyboardKey.escape) {
                seen.add('escape');
              }
            }
            return KeyEventResult.handled;
          },
          child: const SizedBox(width: 10, height: 10),
        ),
      ),
    );
    await tester.pump();
    pressKeyCombo(parseKeyCombo('ctrl+k'));
    pressKeyCombo(parseKeyCombo('escape'));
    expect(seen.first, 'ctrl+k');
    expect(seen, contains('escape'));
    expect(HardwareKeyboard.instance.isControlPressed, isFalse);
  });
}

// The boot path (NotesApp, from the first frame until Home is up) never
// throws and never shows an ErrorWidget: in a release build an ErrorWidget is
// a plain grey box, which is what a throwing build looks like on screen.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/shell/boot.dart';
import 'package:notes_app/shell/home.dart';

import 'harness.dart';

void main() {
  setUpAll(initCore);

  for (final dark in [false, true]) {
    for (final mobile in [false, true]) {
      testWidgets('boot shows no ErrorWidget (dark=$dark mobile=$mobile)', (t) async {
        final dir = Directory.systemTemp.createTempSync('notes-boot-');
        t.view.devicePixelRatio = mobile ? 2.0 : 1.0;
        t.view.physicalSize = mobile ? const Size(750, 1624) : const Size(1440, 900);
        t.platformDispatcher.platformBrightnessTestValue = dark ? Brightness.dark : Brightness.light;
        addTearDown(() {
          t.view.reset();
          t.platformDispatcher.clearAllTestValues();
        });
        final problems = <String>[];
        void check(String when) {
          final e = t.takeException();
          // Layout overflows paint nothing in a release build (test fonts differ).
          if (e != null && !e.toString().contains('overflowed')) problems.add('$when: exception $e');
          for (final w in find.byType(ErrorWidget).evaluate()) {
            problems.add('$when: ErrorWidget ${(w.widget as ErrorWidget).message}');
          }
        }

        await t.pumpWidget(NotesApp(dataDir: dir.path));
        check('first frame');
        var home = false;
        for (var i = 0; i < 400 && !home; i++) {
          await t.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 25)));
          await t.pump(const Duration(milliseconds: 16));
          check('frame $i');
          home = find.byType(HomeScreen).evaluate().isNotEmpty;
        }
        for (var i = 0; i < 20; i++) {
          await t.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 25)));
          await t.pump(const Duration(milliseconds: 100));
          check('after home $i');
        }
        expect(home, isTrue, reason: 'Home never appeared');
        await t.pumpWidget(const SizedBox.shrink());
        await t.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 200)));
        await t.pump(const Duration(seconds: 3));
        expect(problems.toSet().take(8).toList(), isEmpty);
        try {
          dir.deleteSync(recursive: true);
        } catch (_) {}
      });
    }
  }
}

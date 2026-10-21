// What the app remembers across restarts, on the real core: page display
// options (stored in the page, so they sync), and this device's interface
// state (sidebar width, collapse and expanded pages, the open page, open
// toggles) in ui_state.json. Each test restarts the stores on the same
// vault folder with [restartApp].

import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/state/pages_store.dart';
import 'package:notes_app/state/ui_state.dart';

import 'harness.dart';
import 'pages_fixture.dart';

Finder key(String k) => find.byKey(ValueKey(k));

String? _openTitle(WidgetTester tester) {
  final f = key('page-title');
  if (f.evaluate().isEmpty) return null;
  return tester.widget<EditableText>(f).controller.text;
}

void main() {
  setUpAll(initCore);

  group('ui state file', () {
    test('round-trips and drops page state of another vault', () {
      final dir = Directory.systemTemp.createTempSync('notes_ui_');
      addTearDown(() => dir.deleteSync(recursive: true));
      final f = File('${dir.path}/ui_state.json');
      final a = UiState.load(f, 'A')
        ..sidebarWidth = 333
        ..sidebarCollapsed = true
        ..lastPage = 'p1';
      a.expanded.add('p2');
      a.openToggles('p1').add('b1');
      a.flush();

      final again = UiState.load(f, 'A');
      expect(again.sidebarWidth, 333);
      expect(again.sidebarCollapsed, isTrue);
      expect(again.lastPage, 'p1');
      expect(again.expanded, {'p2'});
      expect(again.openToggles('p1'), {'b1'});
      again.openToggles('p1').remove('b1');
      expect(again.openToggles('p1'), isEmpty);

      final other = UiState.load(f, 'B');
      expect(other.sidebarWidth, 333);
      expect(other.lastPage, isNull);
      expect(other.expanded, isEmpty);
      expect(other.openToggles('p1'), isEmpty);

      // A broken file gives the defaults.
      f.writeAsStringSync('{not json');
      expect(UiState.load(f, 'A').sidebarWidth, UiState.defaultSidebarWidth);
    });
  });

  group('restart', () {
    appTest('page options, the open page, toggles and the sidebar come back', (tester) async {
      var s = await pumpApp(tester, mobile: false, persist: true);
      final rich = await createRichPage(tester, s);
      final recipes = s.pages['Recipes']!;

      // Sidebar: wider, Recipes expanded.
      await tester.drag(key('sidebar-resize'), const Offset(60, 0), kind: PointerDeviceKind.mouse);
      await settle(tester);
      s.app.tree.expand(recipes);
      await settle(tester);
      expect(key('tree-${s.pages['Soups']}'), findsOneWidget);

      // The rich page: open its toggle, set every option.
      await openByTitle(tester, richTitle);
      expect(find.text('Passport'), findsNothing);
      await tapAndSettle(tester, key('toggle-r06'));
      expect(editableWith('Passport'), findsOneWidget);
      await tapAndSettle(tester, key('page-menu'));
      await tapAndSettle(tester, key('page-menu-full-width'));
      await tapAndSettle(tester, key('page-menu-small-text'));
      await tester.tapAt(const Offset(800, 880));
      await settle(tester);
      s.app.pages.updatePrefs(rich.id, (p) => p.coverY = 0.2);
      await settle(tester);

      final saved = await pageOf(tester, rich.id);
      expect(saved.prefs.fullWidth, isTrue);
      expect(saved.prefs.smallText, isTrue);
      expect(saved.prefs.locked, isFalse);
      expect(saved.prefs.coverPosition, 200);

      s = await restartApp(tester);
      // The page reopened with its options and its open toggle.
      expect(_openTitle(tester), richTitle);
      expect(editableWith('Passport'), findsOneWidget);
      final PagePrefs p = s.app.pages.prefs(rich.id);
      expect(p.fullWidth, isTrue);
      expect(p.smallText, isTrue);
      expect(p.coverY, closeTo(0.2, 0.001));
      expect(tester.getSize(key('sidebar')).width, closeTo(300, 1));
      expect(key('tree-${s.pages['Soups']}'), findsOneWidget);

      // Close the toggle, go home and collapse the sidebar: that comes back too.
      await tapAndSettle(tester, key('toggle-r06'));
      await tapAndSettle(tester, key('sidebar-home'));
      expect(_openTitle(tester), isNull);
      s.app.tree.setSidebarCollapsed(true);
      await settle(tester);

      s = await restartApp(tester);
      expect(_openTitle(tester), isNull);
      expect(key('sidebar'), findsNothing);
      s.app.tree.setSidebarCollapsed(false);
      await settle(tester);
      await openByTitle(tester, richTitle);
      expect(find.text('Passport'), findsNothing);
    });

    appTest('a last page that went to the trash opens home', (tester) async {
      var s = await pumpApp(tester, mobile: false, persist: true);
      final budget = s.pages['Budget']!;
      await tapAndSettle(tester, key('tree-$budget'));
      expect(_openTitle(tester), 'Budget');
      await tester.runAsync(() => s.vault.deletePage(budget));
      s.app.ui.lastPage = budget; // the editor closing would clear it
      s.app.ui.flush();
      s = await restartApp(tester);
      expect(_openTitle(tester), isNull);
      expect(s.app.ui.lastPage, isNull);
    });
  });
}

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/shell/navigation.dart';
import 'package:notes_app/shell/mobile_page_history.dart';

import 'db_harness.dart' as db;

void main() {
  setUpAll(db.initCore);
  db.dbTest(
    'mobile full-row back restores its database and remembered top page',
    (t) async {
      final s = await db.pumpDb(t, mobile: true);
      await db.openView(t, 'Board', mobile: true);
      final database = t.state<PageEditorState>(
        find.byWidgetPredicate(
          (w) => w is PageScreen && w.pageId == s.projects,
        ),
      );
      final stack = PageStackScope.of(database.context);
      final host = MobilePageHistoryScope.maybeOf(database.context)!;
      final route = stack.pages.single;
      await db.tapKey(t, 'card-Quarterly report');
      // Database rows append inside the same mobile route.
      expect(stack.pages, [route]);
      expect(host.history.entries.length, 2);
      expect(host.history.index, 1);
      expect(stack.topId, isNot(s.projects));
      expect(s.app.ui.lastPage, stack.topId);
      final views = Map.of(host.history.entries.first.databaseViews);
      final rowEditor = host.editor;
      final drag = await t.startGesture(const Offset(80, 50));
      await drag.moveBy(const Offset(20, 0));
      await drag.moveBy(const Offset(80, 0));
      await db.settle(t);
      // The revealing neighbor uses Board too, without overwriting its entry.
      expect(db.byKey('card-Quarterly report'), findsOneWidget);
      expect(s.app.pageStore.session(s.projects), isNull);
      await drag.cancel();
      await db.settle(t);
      expect(host.editor, same(rowEditor));
      expect(host.history.entries.first.databaseViews, views);
      await db.tapKey(t, 'back');
      expect(stack.pages, [route]);
      expect(host.history.index, 0);
      expect(stack.topId, s.projects);
      expect(s.app.ui.lastPage, s.projects);
      // Only the active editor is kept; its database view preference survives.
      expect(
        t.state<PageEditorState>(
          find.byWidgetPredicate(
            (w) => w is PageScreen && w.pageId == s.projects,
          ),
        ),
        isNot(same(database)),
      );
      expect(db.byKey('card-Quarterly report'), findsOneWidget);
    },
  );
}

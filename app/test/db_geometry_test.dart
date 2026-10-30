// Database geometry: the titles of inline and full-page databases against the
// text column, and the right edge of a table's rows against its header. The
// rules are measured on the rendered boxes (desktop 1440x900, phone 375x812).

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/ui/database/values.dart' show ViewDefX;

import 'db_harness.dart';

Finder editableText(String text) => find.byWidgetPredicate(
  (w) => w is EditableText && w.controller.text == text,
);

/// A decorated Container with a top border: the table header (rows have only
/// a bottom border).
bool _hasTopBorder(Widget w) {
  final d = w is Container ? w.decoration : null;
  return d is BoxDecoration &&
      d.border is Border &&
      (d.border! as Border).top.style != BorderStyle.none;
}

Finder _header(String col) => find.ancestor(
  of: byKey('col-$col'),
  matching: find.byWidgetPredicate(_hasTopBorder),
);

Finder _row(String title) => byKey('row-$title');

/// Sets the Table view's visible properties and wrap mode.
Future<void> _view(
  WidgetTester tester, {
  List<String>? props,
  bool? wrap,
}) async {
  final store = dbState(tester).store;
  await tester.runAsync(() async {
    if (props != null) {
      await store.updateView(store.view!.copyWith(visibleProps: props));
    }
    if (wrap != null) {
      await store.updateConfig(store.view!, (c) => c.wrap = wrap);
    }
  });
  await settle(tester);
}

void _expectSameRight(
  WidgetTester tester,
  Finder row,
  Finder header,
  String what,
) {
  final r = tester.getRect(row).right;
  final h = tester.getRect(header).right;
  expect(
    r,
    moreOrLessEquals(h, epsilon: 0.5),
    reason: '$what: row right $r, header right $h',
  );
}

void _expectX(WidgetTester tester, Finder f, double x, String what) {
  final dx = tester.getTopLeft(f).dx;
  expect(
    dx,
    moreOrLessEquals(x, epsilon: 0.5),
    reason: '$what: left $dx, expected $x',
  );
}

/// Drags the table sideways and checks that it moved.
Future<void> _scrollSideways(
  WidgetTester tester,
  Finder row,
  String cellKey,
) async {
  final before = tester.getTopLeft(byKey(cellKey)).dx;
  await tester.dragFrom(visibleCenter(tester, row), const Offset(-300, 0));
  await settle(tester);
  expect(
    tester.getTopLeft(byKey(cellKey)).dx,
    lessThan(before - 1),
    reason: 'the table did not scroll sideways',
  );
}

void main() {
  setUpAll(initCore);

  // ------------------------------------------------------------- titles

  for (final mobile in [false, true]) {
    final device = mobile ? 'phone' : 'desktop';

    dbTest('$device: inline title sits on the paragraph body x', (
      tester,
    ) async {
      await pumpDb(tester, mobile: mobile);
      await openPage(tester, 'Planning');
      _expectX(
        tester,
        byKey('db-inline-title'),
        tester.getTopLeft(editableText('Projects for this quarter:')).dx,
        'inline title vs body',
      );
    });

    // A former database opens as a page: its title sits over the table of
    // its view block.
    dbTest('$device: page title equals the view block table header left', (
      tester,
    ) async {
      await pumpDb(tester, mobile: mobile);
      await openView(tester, 'Table');
      _expectX(
        tester,
        byKey('page-title'),
        tester.getTopLeft(_header('Name')).dx,
        'page title vs header',
      );
    });
  }

  dbTest('phone: inline and page titles both sit at x=18', (tester) async {
    await pumpDb(tester, mobile: true);
    await openPage(tester, 'Planning');
    _expectX(tester, byKey('db-inline-title'), 18, 'inline title');
    _expectX(tester, byKey('page-title'), 18, 'host page title');
    _expectX(
      tester,
      editableText('Projects for this quarter:'),
      18,
      'inline body',
    );
    await openView(tester, 'Table');
    _expectX(tester, byKey('page-title').last, 18, 'page title');
    _expectX(tester, _header('Name'), 18, 'view block table header');
  });

  // ------------------------------------------------- row vs header border

  for (final mobile in [false, true]) {
    final device = mobile ? 'phone' : 'desktop';

    dbTest('$device full-page: row border right equals header right', (
      tester,
    ) async {
      await pumpDb(tester, mobile: mobile);
      await openView(tester, 'Table');
      _expectSameRight(
        tester,
        _row('Website redesign'),
        _header('Name'),
        'full page, all columns',
      );
    });

    dbTest('$device full-page: compact table narrower than the viewport', (
      tester,
    ) async {
      await pumpDb(tester, mobile: mobile);
      await openView(tester, 'Table');
      await _view(tester, props: [db.schema['Status']!.key]);
      _expectSameRight(
        tester,
        _row('Website redesign'),
        _header('Name'),
        'full page, compact',
      );
    });

    dbTest('$device full-page: wrapped rows keep the header right', (
      tester,
    ) async {
      await pumpDb(tester, mobile: mobile);
      await openView(tester, 'Table');
      await _view(tester, wrap: true);
      _expectSameRight(
        tester,
        _row('Website redesign'),
        _header('Name'),
        'full page, wrapped',
      );
    });

    dbTest('$device inline: row border right equals header right', (
      tester,
    ) async {
      await pumpDb(tester, mobile: mobile);
      await openPage(tester, 'Planning');
      _expectSameRight(
        tester,
        _row('Website redesign'),
        _header('Name'),
        'inline, all columns',
      );
    });

    dbTest('$device inline: compact table narrower than the block', (
      tester,
    ) async {
      await pumpDb(tester, mobile: mobile);
      await openPage(tester, 'Planning');
      await _view(tester, props: [db.schema['Status']!.key]);
      _expectSameRight(
        tester,
        _row('Website redesign'),
        _header('Name'),
        'inline, compact',
      );
    });

    dbTest('$device inline: wrapped rows keep the header right', (
      tester,
    ) async {
      await pumpDb(tester, mobile: mobile);
      await openPage(tester, 'Planning');
      await _view(tester, wrap: true);
      _expectSameRight(
        tester,
        _row('Website redesign'),
        _header('Name'),
        'inline, wrapped',
      );
    });
  }

  // ----------------------------------------------------- sideways scroll

  dbTest(
    'desktop full-page: the table scrolls and the header stays with the rows',
    (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      await _scrollSideways(
        tester,
        _row('Website redesign'),
        'cell-Website redesign-Status',
      );
      _expectSameRight(
        tester,
        _row('Website redesign'),
        _header('Name'),
        'full page, scrolled',
      );
    },
  );

  dbTest(
    'desktop inline: the table scrolls and the header stays with the rows',
    (tester) async {
      await pumpDb(tester, mobile: false);
      await openPage(tester, 'Planning');
      await _scrollSideways(
        tester,
        _row('Website redesign'),
        'cell-Website redesign-Status',
      );
      _expectSameRight(
        tester,
        _row('Website redesign'),
        _header('Name'),
        'inline, scrolled',
      );
    },
  );
}

// The page menu's Share item: shown only with a hub sign-in, and it opens the
// Share popup. Needs the full app and the Rust bridge.

import 'package:flutter_test/flutter_test.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:notes_app/features/share/share_popup.dart';

import 'harness.dart';

Finder key(String k) => find.byKey(ValueKey(k));

void main() {
  setUpAll(initCore);

  setUp(() => SharePopup.debugClientFor = null);
  tearDown(() => SharePopup.debugClientFor = null);

  appTest('the Share item shows only with a hub sign-in', (t) async {
    final tv = await pumpApp(t, mobile: false);
    await openShowcase(t);

    await tapAndSettle(t, key('page-menu'));
    expect(key('page-menu-copy-link'), findsOneWidget);
    expect(key('page-menu-share'), findsNothing);
    await t.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle(t);

    tv.app.settings.update(
      (s) => s
        ..clientUrl = 'https://hub.example.com'
        ..clientToken = 'test-token',
    );
    addTearDown(
      () => tv.app.settings.update(
        (s) => s
          ..clientUrl = null
          ..clientToken = null,
      ),
    );
    await tapAndSettle(t, key('page-menu'));
    expect(key('page-menu-share'), findsOneWidget);
  });

  appTest('Share from the menu opens the popup for the page', (t) async {
    final tv = await pumpApp(t, mobile: false);
    tv.app.settings.update(
      (s) => s
        ..clientUrl = 'https://hub.example.com'
        ..clientToken = 'test-token',
    );
    SharePopup.debugClientFor = (_) => null;
    await openShowcase(t);
    await tapAndSettle(t, key('page-menu'));
    await tapAndSettle(t, key('page-menu-share'));
    expect(key('share-signed-out'), findsOneWidget);
  });
}

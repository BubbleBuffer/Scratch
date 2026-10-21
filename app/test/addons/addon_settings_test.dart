import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/settings/controls.dart';
import 'package:notes_app/registry/view_registry.dart';
import 'package:notes_app/core/models.dart';

import '../harness.dart';

Finder key(String value) => find.byKey(ValueKey(value));

void main() {
  setUpAll(initCore);
  for (final mobile in [false, true]) {
    appTest(
      'About manages optional add-ons on ${mobile ? 'mobile' : 'desktop'}',
      (tester) async {
        final tv = await pumpApp(tester, mobile: mobile);
        if (mobile) {
          await openMobileSettings(tester);
        } else {
          await tapAndSettle(tester, key('sidebar-settings'));
        }
        await tapAndSettle(tester, key('settings-nav-about'));
        await tapAndSettle(tester, key('manage-addons'));
        expect(
          find.text('Changes take effect when you restart Scratch.'),
          findsOneWidget,
        );
        expect(key('addon-database'), findsNothing);
        expect(key('addon-document'), findsNothing);
        expect(key('addon-legacy'), findsNothing);
        expect(tester.widget<Toggle>(key('addon-board')).value, isTrue);
        await tapAndSettle(tester, key('addon-board'));
        expect(tv.app.settings.disabledAddons, {'board'});
        expect(tester.widget<Toggle>(key('addon-board')).value, isFalse);
        // The current process keeps its active view until the documented restart.
        expect(viewRegistry.of(ViewKind.board), isNotNull);
        await tapAndSettle(tester, key('addon-board'));
        expect(tv.app.settings.disabledAddons, isEmpty);
        expect(tester.widget<Toggle>(key('addon-board')).value, isTrue);
      },
    );
  }
}

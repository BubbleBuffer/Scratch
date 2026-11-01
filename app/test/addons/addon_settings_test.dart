import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import '../harness.dart';

Finder key(String value) => find.byKey(ValueKey(value));

void main() {
  setUpAll(initCore);
  for (final mobile in [false, true]) {
    appTest(
      'About has no add-on manage entry on ${mobile ? 'mobile' : 'desktop'}',
      (tester) async {
        await pumpApp(tester, mobile: mobile);
        if (mobile) {
          await openMobileSettings(tester);
        } else {
          await tapAndSettle(tester, key('sidebar-settings'));
        }
        await tapAndSettle(tester, key('settings-nav-about'));
        expect(key('device-id'), findsOneWidget);
        expect(key('manage-addons'), findsNothing);
        expect(find.text('Add-ons'), findsNothing);
      },
    );
  }
}

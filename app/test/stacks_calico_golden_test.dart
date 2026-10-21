@Tags(['golden'])
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/shell/navigation.dart';
import 'package:notes_app/ui/confetti.dart';

import 'harness.dart';
import 'stacks_calico_fixture.dart';

void main() {
  setUpAll(initCore);
  for (final dark in [false, true]) {
    final theme = dark ? 'dark' : 'light';
    appTest('Calico coloured page $theme', (tester) async {
      await pumpApp(tester, mobile: false, dark: dark, empty: true);
      final f = await seedCalico(tester);
      await openCalico(tester, f.page, 'Weekend reading');
      await calicoShot('desktop-$theme-page');
    });
    appTest('Calico three coloured sheets $theme', (tester) async {
      await pumpApp(tester, mobile: false, dark: dark, empty: true);
      final f = await seedCalico(tester);
      await openCalico(tester, f.page, 'Weekend reading');
      await openCalico(tester, f.target, 'Trips', from: f.page);
      await openCalico(tester, f.empty, 'A quiet page', from: f.target);
      await calicoShot('desktop-$theme-stack3');
    });
    appTest('Calico Home recent folder tabs $theme', (tester) async {
      await pumpApp(tester, mobile: false, dark: dark, empty: true);
      final f = await seedCalico(tester);
      await openCalico(tester, f.page, 'Weekend reading');
      goHome(calicoEditor(tester, f.page).context);
      await settle(tester);
      await calicoShot('desktop-$theme-home');
    });
    appTest('Calico empty page cat $theme', (tester) async {
      await pumpApp(tester, mobile: false, dark: dark, empty: true);
      final f = await seedCalico(tester);
      await openCalico(tester, f.empty, 'A quiet page');
      await calicoShot('desktop-$theme-empty');
    });
    appTest('Calico mobile coloured page $theme', (tester) async {
      await pumpApp(tester, mobile: true, dark: dark, empty: true);
      final f = await seedCalico(tester);
      await openCalico(tester, f.page, 'Weekend reading');
      await calicoShot('mobile-$theme-page');
    });
    appTest('Calico mobile Home folder tabs $theme', (tester) async {
      await pumpApp(tester, mobile: true, dark: dark, empty: true);
      final f = await seedCalico(tester);
      await openCalico(tester, f.page, 'Weekend reading');
      goHome(calicoEditor(tester, f.page).context);
      await settle(tester);
      await Scrollable.ensureVisible(
        tester.element(find.byKey(ValueKey('home-folder-tab-${f.page}'))),
        alignment: .3,
      );
      await settle(tester);
      await calicoShot('mobile-$theme-home');
    });
    appTest('Calico mobile empty cat $theme', (tester) async {
      await pumpApp(tester, mobile: true, dark: dark, empty: true);
      final f = await seedCalico(tester);
      await openCalico(tester, f.empty, 'A quiet page');
      await calicoShot('mobile-$theme-empty');
    });
  }
  appTest('Calico checklist completion at a fixed confetti frame', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final f = await seedCalico(tester);
    await openCalico(tester, f.page, 'Weekend reading');
    await tester.tap(find.byKey(const ValueKey('check-calico-two')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 250));
    final state = tester.state<ConfettiBurstState>(
      find.byKey(const ValueKey('heading-confetti-calico-heading')),
    );
    expect(state.animation.value, closeTo(250 / 900, .001));
    await calicoShot('desktop-light-complete-confetti');
  });
}

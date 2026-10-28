// PageStackChrome before the vault opens: there is no store scope yet, so on
// desktop it must show the child alone instead of looking up the TreeStore.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/shell/page_stack.dart';
import 'package:notes_app/ui/ui.dart';

void main() {
  testWidgets('desktop with no store scope shows the child and no pile', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(1440, 900);
    addTearDown(tester.view.reset);

    // The app's theme root (app_shell.dart) is always above the chrome; only
    // the store scopes are missing before the vault opens.
    await tester.pumpWidget(
      MediaQuery(
        data: const MediaQueryData(size: Size(1440, 900)),
        child: AppTheme(
          colors: AppColors.light,
          formFactor: FormFactor.desktop,
          child: Directionality(
            textDirection: TextDirection.ltr,
            child: PageStackChrome(
              child: const SizedBox(
                key: ValueKey('desk-child'),
                width: 10,
                height: 10,
              ),
            ),
          ),
        ),
      ),
    );

    expect(tester.takeException(), isNull);
    expect(find.byKey(const ValueKey('desk-child')), findsOneWidget);
    expect(find.byKey(const ValueKey('stack-fan-barrier')), findsNothing);
  });
}

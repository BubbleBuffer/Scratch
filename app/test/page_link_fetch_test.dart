// A page link to a page this device lacks asks the hub for it (fake client,
// cert_helpers.dart, on the real app). The page "arrives" when the fake's
// fetch restores a trashed page, since a trashed page counts as missing:
// - "Loading page" with Cancel while the hub answers, then the page opens
//   and a sync is requested;
// - offline (also a timeout) shows a message and Retry;
// - the hub not having the page ends in the existing not-found toast;
// - Unauthorized offers Sign in, then fetches again;
// - an untrusted certificate shows the existing certificate prompt;
// - with no hub set up nothing is shown but the not-found toast.

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/page_link.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/features/sync/sync_control.dart';
import 'package:notes_app/shell/navigation.dart';
import 'package:notes_app/shell/page_link_fetch.dart';
import 'package:notes_app/state/sync_store.dart';

import 'cert_helpers.dart';
import 'harness.dart';
import 'sync_helpers.dart';

/// A client whose `fetchPages` is scripted.
class ScriptedSync extends FakeSync {
  ScriptedSync(super.hubs, super.trusted, this.script);
  final Future<PageFetch> Function(List<String> ids) script;
  int syncNows = 0;

  @override
  Future<PageFetch> fetchPages({required List<String> ids}) => script(ids);

  @override
  void syncNow() => syncNows++;
}

/// Waits until the step shows [title]: one quiet frame can come before a
/// scripted fetch that awaits a native import has answered.
Future<void> stepIs(WidgetTester tester, String title) => settleUntil(
  tester,
  () => key('link-fetch-title').evaluate().isNotEmpty && textOf(tester, 'link-fetch-title') == title,
);

PageFetch result(PageFetchStatus s, [String message = '']) => PageFetch(status: s, message: message);

void main() {
  setUpAll(initCore);
  tearDown(() {
    FakeHubs.uninstall();
    final s = AppSettings.memory;
    s.clientUrl = null;
    s.clientToken = null;
    s.hubSignInUrl = '';
  });

  /// The app, a trashed page (missing for links) and a client whose fetch
  /// runs [script]; [restore] brings the page back, as the hub's copy would.
  Future<(String, ScriptedSync Function(), Future<void> Function())> setup(
    WidgetTester tester,
    Future<PageFetch> Function(List<String> ids, Future<void> Function() arrive) script,
  ) async {
    await pumpApp(tester, mobile: false);
    // The page lives in another vault (the "hub"); it reaches this one as the
    // snapshot a fetch would import.
    final other = await openOtherDevice(tester);
    final id = await real(tester, () => other.vault.createPage(title: 'From the hub'));
    final bytes = await real(tester, () => other.vault.raw.exportUpdates(page: id, since: const []));
    await other.close(tester);
    Future<void> arrive() => current.app.vault.run((v) => v.importUpdates(page: id, bytes: bytes), writes: true);
    late ScriptedSync client;
    final hubs = FakeHubs();
    SyncStore.clientFactory = (v, url, token, trusted) =>
        client = ScriptedSync(hubs, [for (final t in trusted) t.sha256], (ids) => script(ids, arrive));
    final s = AppSettings.read(homeContext(tester));
    await connectToHub(current.app.sync, s, hubUrl, 'ab' * 32);
    await settle(tester);
    return (id, () => client, arrive);
  }

  appTest('a missing page shows "Loading page" with Cancel, then opens and syncs', (tester) async {
    final gate = Completer<PageFetch>();
    final (id, client, arrive) = await setup(tester, (ids, arrive) async {
      await arrive();
      return gate.future;
    });
    unawaited(openPageLink(homeContext(tester), PageLink(id)));
    await settle(tester);
    await stepIs(tester, 'Loading page');
    expect(textOf(tester, 'link-fetch-title'), 'Loading page');
    expect(key('link-fetch-cancel'), findsOneWidget);
    expect(key('link-fetch-retry'), findsNothing);

    gate.complete(result(PageFetchStatus.fetched));
    // The fetch script awaits a native import: wait for the step to close
    // rather than for a quiet frame, which can come first under load.
    await settleUntil(tester, () => key('link-fetch-title').evaluate().isEmpty);
    expect(key('link-fetch-title'), findsNothing);
    expect(current.app.ui.lastPage, id);
    expect(client().syncNows, greaterThan(0));
  });

  appTest('Cancel closes the step and shows nothing else', (tester) async {
    final gate = Completer<PageFetch>();
    final (id, _, _) = await setup(tester, (ids, arrive) => gate.future);
    unawaited(openPageLink(homeContext(tester), PageLink(id)));
    await settle(tester);
    await tapAndSettle(tester, key('link-fetch-cancel'));
    expect(key('link-fetch-title'), findsNothing);
    expect(find.text(PageLink.missingMessage), findsNothing);
    expect(current.app.ui.lastPage, isNot(id));
  });

  appTest('offline shows a message and Retry fetches again', (tester) async {
    var calls = 0;
    final (id, _, _) = await setup(tester, (ids, arrive) async {
      if (++calls == 1) return result(PageFetchStatus.offline);
      await arrive();
      return result(PageFetchStatus.fetched);
    });
    unawaited(openPageLink(homeContext(tester), PageLink(id)));
    await settle(tester);
    await stepIs(tester, 'Hub unreachable');
    expect(textOf(tester, 'link-fetch-title'), 'Hub unreachable');
    expect(textOf(tester, 'link-fetch-message'), "Can't reach the hub.");
    await tapAndSettle(tester, key('link-fetch-retry'));
    await settleUntil(tester, () => key('link-fetch-title').evaluate().isEmpty);
    expect(calls, 2);
    expect(key('link-fetch-title'), findsNothing);
    expect(current.app.ui.lastPage, id);
  });

  appTest('a hub that does not answer in time counts as offline', (tester) async {
    final (id, _, _) = await setup(tester, (ids, arrive) => Completer<PageFetch>().future);
    unawaited(fetchLinkedPage(homeContext(tester), PageLink(id), timeout: const Duration(seconds: 2)));
    await settle(tester);
    await stepIs(tester, 'Loading page');
    expect(textOf(tester, 'link-fetch-title'), 'Loading page');
    await tester.pump(const Duration(seconds: 3));
    await settle(tester);
    await stepIs(tester, 'Hub unreachable');
    expect(textOf(tester, 'link-fetch-title'), 'Hub unreachable');
    expect(key('link-fetch-retry'), findsOneWidget);
  });

  appTest('a page the hub lacks ends in the not-found toast', (tester) async {
    final (id, _, _) = await setup(tester, (ids, arrive) async => result(PageFetchStatus.notOnHub));
    unawaited(openPageLink(homeContext(tester), PageLink(id)));
    await settle(tester);
    expect(key('link-fetch-title'), findsNothing);
    expect(find.text(PageLink.missingMessage), findsOneWidget);
    expect(current.app.ui.lastPage, isNot(id));
  });

  appTest('Unauthorized offers Sign in, then fetches again', (tester) async {
    var calls = 0;
    final (id, _, _) = await setup(tester, (ids, arrive) async {
      if (++calls == 1) return result(PageFetchStatus.unauthorized);
      await arrive();
      return result(PageFetchStatus.fetched);
    });
    var signedIn = 0;
    unawaited(fetchLinkedPage(homeContext(tester), PageLink(id), signIn: () async => signedIn++));
    await settle(tester);
    await stepIs(tester, 'Sign in needed');
    expect(textOf(tester, 'link-fetch-title'), 'Sign in needed');
    await tapAndSettle(tester, key('link-fetch-sign-in'));
    await settleUntil(tester, () => key('link-fetch-title').evaluate().isEmpty);
    expect(signedIn, 1);
    expect(calls, 2);
    expect(key('link-fetch-title'), findsNothing);
  });

  appTest('an untrusted certificate shows the existing certificate prompt', (tester) async {
    final (id, _, _) = await setup(
      tester,
      (ids, arrive) async => result(PageFetchStatus.error, 'untrusted certificate: $leafSha'),
    );
    unawaited(openPageLink(homeContext(tester), PageLink(id)));
    await settle(tester);
    await stepIs(tester, 'Certificate not trusted');
    expect(textOf(tester, 'link-fetch-title'), 'Certificate not trusted');
    expect(key('cert-view'), findsOneWidget);
    expect(key('cert-trust'), findsOneWidget);
    await tapAndSettle(tester, key('link-fetch-cancel'));
    expect(key('link-fetch-title'), findsNothing);
  });

  appTest('a page in the trash stays missing: toast, no fetch, no sign-in prompt', (tester) async {
    var calls = 0;
    final (_, _, _) = await setup(tester, (ids, arrive) async {
      calls++;
      return result(PageFetchStatus.fetched);
    });
    final vault = current.app.vault;
    final id = await real(tester, () async {
      final id = await vault.createPage(title: 'Trashed');
      await vault.deletePage(id);
      return id;
    });
    unawaited(openPageLink(homeContext(tester), PageLink(id)));
    await settle(tester);
    expect(key('link-fetch-title'), findsNothing);
    expect(find.text(PageLink.missingMessage), findsOneWidget);
    expect(calls, 0);
  });

  appTest('with no hub set up only the not-found toast shows', (tester) async {
    await pumpApp(tester, mobile: false);
    const id = '01M5CZCX81NZWZM4KEBKTR9SJN';
    unawaited(openPageLink(homeContext(tester), PageLink(id)));
    await settle(tester);
    expect(key('link-fetch-title'), findsNothing);
    expect(find.text(PageLink.missingMessage), findsOneWidget);
  });
}

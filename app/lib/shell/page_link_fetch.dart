// A page link to a page this device does not have yet: ask the hub for it.
// A modal shows "Loading page" while SyncStore.fetchPages pulls the page and
// its missing parents; failures name their state (offline, signed out,
// certificate) with the action that fixes it.

import 'dart:async';

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../core/page_link.dart';
import '../features/settings/settings_store.dart';
import '../features/sync/cert_trust.dart';
import '../features/sync/certificate_view.dart';
import '../features/sync/hub_auth.dart';
import '../features/sync/sync_control.dart';
import '../state/app_state.dart';
import '../state/sync_store.dart';
import '../ui/ui.dart';

/// How the "Loading page" step ended.
enum LinkFetch {
  /// The page is here now.
  fetched,

  /// The hub has no such page (or it is in the trash there).
  notFound,

  /// The user closed the step, or no hub is set up.
  cancelled,
}

/// How long the hub gets to answer before the step reports it as offline.
const linkFetchTimeout = Duration(seconds: 10);

/// Fetches the page of [link] from the hub, showing the "Loading page" step.
/// With no hub set up (never signed in, no pairing) it returns
/// [LinkFetch.cancelled] at once, so the caller keeps its plain not-found
/// behaviour.
Future<LinkFetch> fetchLinkedPage(
  BuildContext context,
  PageLink link, {
  Duration timeout = linkFetchTimeout,
  Future<void> Function()? signIn,
}) async {
  final sync = context.read<SyncStore>();
  final settings = AppSettings.read(context);
  if (!sync.connected && settings.hubSignInUrl.trim().isEmpty) {
    return LinkFetch.cancelled;
  }
  Widget body(BuildContext ctx) => LinkFetchPanel(
    pageId: link.pageId,
    sync: sync,
    settings: settings,
    timeout: timeout,
    signIn: signIn,
    onDone: (r) => Navigator.of(ctx).pop(r),
  );
  final result = context.mobile
      ? await showSheet<LinkFetch>(
          context,
          (ctx) => SheetSurface(children: [body(ctx)]),
        )
      : await showModalPanel<LinkFetch>(context, body, width: 420, shrink: true);
  return result ?? LinkFetch.cancelled;
}

enum _Step { loading, offline, signedOut, signingIn, certificate, failed }

/// The step's content; [onDone] closes it with the outcome.
class LinkFetchPanel extends StatefulWidget {
  const LinkFetchPanel({
    super.key,
    required this.pageId,
    required this.sync,
    required this.settings,
    required this.onDone,
    this.timeout = linkFetchTimeout,
    this.signIn,
  });

  final String pageId;
  final SyncStore sync;
  final AppSettings settings;
  final void Function(LinkFetch) onDone;
  final Duration timeout;

  /// Signs in to the hub; the default is the Sync settings flow.
  final Future<void> Function()? signIn;

  @override
  State<LinkFetchPanel> createState() => _LinkFetchPanelState();
}

class _LinkFetchPanelState extends State<LinkFetchPanel> {
  _Step _step = _Step.loading;
  String? _message;
  int _attempt = 0;
  HubAuthService? _auth;

  @override
  void initState() {
    super.initState();
    _start();
  }

  @override
  void dispose() {
    _attempt++;
    _auth?.cancel();
    _auth?.dispose();
    super.dispose();
  }

  void _set(_Step step, [String? message]) {
    if (!mounted) return;
    setState(() {
      _step = step;
      _message = message;
    });
  }

  Future<void> _start() async {
    final attempt = ++_attempt;
    bool stale() => !mounted || attempt != _attempt;
    _set(_Step.loading);
    if (!widget.sync.connected) {
      _set(_Step.signedOut);
      return;
    }
    PageFetch? result;
    try {
      result = await widget.sync
          .fetchPages([widget.pageId])
          .timeout(widget.timeout);
    } on TimeoutException {
      if (!stale()) _set(_Step.offline);
      return;
    } catch (e) {
      if (!stale()) _set(_Step.failed, '$e');
      return;
    }
    if (stale()) return;
    switch (result?.status) {
      case PageFetchStatus.fetched:
        widget.onDone(LinkFetch.fetched);
      case PageFetchStatus.notOnHub:
        widget.onDone(LinkFetch.notFound);
      case PageFetchStatus.offline:
        _set(_Step.offline);
      case PageFetchStatus.unauthorized:
        _set(_Step.signedOut);
      case PageFetchStatus.error:
        final m = result!.message;
        if (isUntrustedCertificateError(m)) {
          // The background loop meets the same certificate; ask it so the
          // existing prompt knows the fingerprint.
          widget.sync.syncNow();
          _set(_Step.certificate, m);
        } else {
          _set(_Step.failed, m);
        }
      case null:
        _set(_Step.signedOut);
    }
  }

  Future<void> _signIn() async {
    _set(_Step.signingIn);
    final auth = _auth ??= HubAuthService();
    try {
      final custom = widget.signIn;
      if (custom != null) {
        await custom();
      } else {
        final url = widget.sync.hubUrl ?? widget.settings.hubSignInUrl.trim();
        await signInToHub(widget.sync, widget.settings, auth, url);
      }
    } catch (e) {
      _set(
        _Step.signedOut,
        e is HubAuthException ? e.message : 'Could not sign in to the hub.',
      );
      return;
    }
    if (mounted) await _start();
  }

  void _cancel() {
    if (_step == _Step.signingIn) _auth?.cancel();
    widget.onDone(LinkFetch.cancelled);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final (title, detail) = switch (_step) {
      _Step.loading => ('Loading page', 'Fetching it from the hub.'),
      _Step.offline => ('Hub unreachable', "Can't reach the hub."),
      _Step.signedOut => ('Sign in needed', 'Sign in to the hub to open this page.'),
      _Step.signingIn => ('Signing in', 'Waiting for sign in.'),
      _Step.certificate => ('Certificate not trusted', ''),
      _Step.failed => ('Page not loaded', _message ?? ''),
    };
    final note = _step == _Step.signedOut ? _message : null;
    return Padding(
      padding: const EdgeInsets.fromLTRB(20, 18, 20, 16),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            title,
            key: const ValueKey('link-fetch-title'),
            style: tsDisplay(const TypeSpec(19, 24, 640), c.textPrimary),
          ),
          const SizedBox(height: 10),
          if (_step == _Step.certificate)
            SyncCertificateProblem(
              messageKey: const ValueKey('link-fetch-message'),
              onTrust: () async {
                if (await trustSyncCertificate(context)) await _start();
              },
            )
          else
            Text(
              detail,
              key: const ValueKey('link-fetch-message'),
              style: tsRaw(13, 18, 400, c.textSecondary),
            ),
          if (note != null) ...[
            const SizedBox(height: 6),
            Text(
              note,
              key: const ValueKey('link-fetch-note'),
              style: tsRaw(12, 16, 400, c.inlineCodeText),
            ),
          ],
          const SizedBox(height: 14),
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              TextBtn(
                'Cancel',
                key: const ValueKey('link-fetch-cancel'),
                onTap: _cancel,
                color: c.textSecondary,
              ),
              if (_step == _Step.offline || _step == _Step.failed) ...[
                const SizedBox(width: 8),
                PrimaryButton(
                  key: const ValueKey('link-fetch-retry'),
                  label: 'Retry',
                  onTap: _start,
                ),
              ],
              if (_step == _Step.signedOut) ...[
                const SizedBox(width: 8),
                PrimaryButton(
                  key: const ValueKey('link-fetch-sign-in'),
                  label: 'Sign in',
                  onTap: _signIn,
                ),
              ],
            ],
          ),
        ],
      ),
    );
  }
}

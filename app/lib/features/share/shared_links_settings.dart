// Settings › Shared links: every share link this device's hub holds, across
// pages (GET /shares?all=1), deleted pages included. Each row has Copy, Open
// note and Revoke, the same row as the Share popup's list. The section is
// listed only while the device is signed in to a hub (see
// SettingsSection.visibleFor).

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../shell/navigation.dart';
import '../../ui/ui.dart';
import '../settings/settings_kit.dart';
import 'share_client.dart';
import 'share_popup.dart' show ShareLinkRow, shareClientOf;

class SharedLinksBody extends StatefulWidget {
  const SharedLinksBody({super.key});

  @override
  State<SharedLinksBody> createState() => _SharedLinksBodyState();
}

class _SharedLinksBodyState extends State<SharedLinksBody> {
  ShareClient? _client;
  bool _started = false;

  /// Every link across pages, newest first. Null until the list arrives.
  List<Share>? _links;
  String? _error;

  /// The share whose row asks "Revoke this link?", waiting for a second press.
  String? _confirming;

  /// The share whose Copy button reads "Copied" for two seconds.
  String? _copiedId;
  Timer? _copyTimer;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_started) return;
    _started = true;
    _client = shareClientOf(context);
    _load();
  }

  @override
  void dispose() {
    _copyTimer?.cancel();
    super.dispose();
  }

  Future<void> _load() async {
    final client = _client;
    if (client == null) return;
    try {
      final links = await client.listAll();
      if (!mounted || !identical(client, _client)) return;
      setState(() {
        _links = links;
        _error = null;
      });
    } on ShareException catch (e) {
      if (mounted && identical(client, _client)) {
        setState(() => _error = e.message);
      }
    }
  }

  Future<void> _copy(Share s) async {
    await Clipboard.setData(ClipboardData(text: s.url));
    if (!mounted) return;
    _copyTimer?.cancel();
    setState(() => _copiedId = s.id);
    _copyTimer = Timer(const Duration(seconds: 2), () {
      if (mounted) setState(() => _copiedId = null);
    });
  }

  Future<void> _revoke(Share s) async {
    final client = _client;
    if (client == null) return;
    setState(() {
      _error = null;
      _confirming = null;
    });
    try {
      await client.revoke(s.id);
      if (!mounted) return;
      setState(() => _links = _links?.where((x) => x.id != s.id).toList());
    } on ShareException catch (e) {
      if (mounted) setState(() => _error = e.message);
    }
  }

  /// Opens the page, the way a page link does. From the settings panel the
  /// panel closes too, so the page is not left beneath it (as conflicts do).
  void _open(Share s) {
    final title = s.title;
    if (title == null) return;
    final nav = Navigator.of(context);
    final panel = ModalRoute.of(context);
    unawaited(openPage(context, s.page, title));
    if (panel is PopupRoute) nav.removeRoute(panel);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final client = _client;
    if (client == null) {
      return SPage(
        title: 'Shared links',
        children: [
          Text(
            'Sign in to a hub in Sync and devices to share pages.',
            key: const ValueKey('shared-links-signed-out'),
            style: sHint(context),
          ),
        ],
      );
    }
    final links = _links;
    return SPage(
      title: 'Shared links',
      intro: 'Pages you have shared from this vault. Revoking a link stops it working for everyone.',
      children: [
        if (_error != null)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: Text(
              _error!,
              key: const ValueKey('shared-links-error'),
              style: tsRaw(12, 16, 400, c.inlineCodeText),
            ),
          ),
        if (links == null && _error == null)
          Text(
            'Loading links…',
            key: const ValueKey('shared-links-loading'),
            style: tsRaw(12, 16, 400, c.textSecondary),
          )
        else if (links != null && links.isEmpty)
          Text(
            'No shared links yet. Share a note from its menu.',
            key: const ValueKey('shared-links-empty'),
            style: tsRaw(13, 18, 400, c.textSecondary),
          )
        else if (links != null)
          for (final s in links)
            ShareLinkRow(
              share: s,
              showTitle: true,
              copied: _copiedId == s.id,
              confirming: _confirming == s.id,
              onCopy: () => _copy(s),
              onAskRevoke: () => setState(() => _confirming = s.id),
              onCancelRevoke: () => setState(() => _confirming = null),
              onRevoke: () => _revoke(s),
              onOpen: s.title == null ? null : () => _open(s),
            ),
      ],
    );
  }
}

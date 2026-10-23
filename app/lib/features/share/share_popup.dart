// Share a page: the "Share" popup of the page menu. It makes and manages
// the page's share links on the hub: public or private, a snapshot of the
// page or a live link. A centred panel on desktop, a sheet on phones.
// Shown only for a document page while this device is signed in to a hub.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../state/app_state.dart';
import '../../ui/database/widgets.dart' show ChipBtn;
import '../../ui/toast.dart';
import '../../ui/ui.dart';
import '../ai/agent_token_client.dart' show AgentTokenClient;
import '../settings/controls.dart' show CopyableText;
import 'share_client.dart';

/// Builds the client for this device, or null when it is not signed in to
/// an https:// hub. Tests pass their own.
typedef ShareClientFactory = ShareClient? Function(BuildContext);

ShareClient? defaultShareClient(BuildContext context) {
  final app = context.app;
  final s = app.settings;
  final url = s.clientUrl?.trim() ?? '';
  final token = s.clientToken?.trim() ?? '';
  if (token.isEmpty || AgentTokenClient.hubUri(url) == null) return null;
  return ShareClient(
    hubUrl: url,
    token: token,
    deviceId: app.vault.deviceId,
  );
}

/// Opens the share popup for [pageId]: a centred panel on desktop, a sheet
/// on a phone.
Future<void> showSharePopup(
  BuildContext context, {
  required String pageId,
  required String pageTitle,
}) {
  Widget body(BuildContext ctx) =>
      SharePopup(pageId: pageId, pageTitle: pageTitle);
  if (context.mobile) {
    return showSheet<void>(
      context,
      (ctx) => SheetSurface(children: [body(ctx)]),
    );
  }
  return showModalPanel<void>(
    context,
    body,
    width: 480,
    height: 640,
    shrink: true,
  );
}

class SharePopup extends StatefulWidget {
  const SharePopup({super.key, required this.pageId, required this.pageTitle});

  final String pageId;
  final String pageTitle;

  /// Replaces [defaultShareClient] everywhere (tests).
  static ShareClientFactory? debugClientFor;

  @override
  State<SharePopup> createState() => _SharePopupState();
}

class _SharePopupState extends State<SharePopup> {
  ShareClient? _client;
  bool _started = false;

  /// The page's active links, newest first. Null until the list arrives.
  List<Share>? _links;
  bool _busy = false;
  String _visibility = 'private';
  String _kind = 'snapshot';
  String? _error;

  /// The link just created, shown in a copyable field until revoked.
  Share? _created;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_started) return;
    _started = true;
    _client = (SharePopup.debugClientFor ?? defaultShareClient)(context);
    _load();
  }

  Future<void> _load() async {
    final client = _client;
    if (client == null) return;
    try {
      final links = await client.list(widget.pageId);
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

  Future<void> _create() async {
    final client = _client;
    if (client == null || _busy) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final share = await client.create(
        widget.pageId,
        visibility: _visibility,
        kind: _kind,
      );
      if (!mounted) return;
      await Clipboard.setData(ClipboardData(text: share.url));
      if (!mounted) return;
      setState(() {
        _created = share;
        final links = _links;
        if (links != null) _links = [share, ...links];
      });
      if (_links == null) _load();
      showToast(context, 'Link copied');
    } on ShareException catch (e) {
      if (mounted) setState(() => _error = e.message);
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _copy(Share s) async {
    await Clipboard.setData(ClipboardData(text: s.url));
    if (mounted) showToast(context, 'Link copied');
  }

  Future<void> _revoke(Share s) async {
    final client = _client;
    if (client == null) return;
    setState(() => _error = null);
    try {
      await client.revoke(s.id);
      if (!mounted) return;
      setState(() {
        _links = _links?.where((x) => x.id != s.id).toList();
        if (_created?.id == s.id) _created = null;
      });
    } on ShareException catch (e) {
      if (mounted) setState(() => _error = e.message);
    }
  }

  Widget _choice(
    BuildContext context, {
    required String group,
    required List<(String, String)> options,
    required String current,
    required ValueChanged<String> onPick,
  }) {
    final h = context.mobile ? 32.0 : 24.0;
    return Row(
      children: [
        for (final (value, label) in options) ...[
          ChipBtn(
            key: ValueKey('share-$group-$value'),
            label: label,
            active: current == value,
            height: h,
            onTap: () => onPick(value),
          ),
          const SizedBox(width: 6),
        ],
      ],
    );
  }

  Widget _hint(BuildContext context, String text) => Padding(
    padding: const EdgeInsets.only(top: 6, bottom: 12),
    child: Text(
      text,
      style: tsRaw(12, 16, 400, context.colors.textSecondary),
    ),
  );

  Widget _row(BuildContext context, Share s) {
    final c = context.colors;
    return Padding(
      key: ValueKey('share-row-${s.id}'),
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(s.label, style: tsRaw(14, 20, 400, c.textPrimary)),
                Text(
                  relativeTime(s.created),
                  style: tsRaw(12, 16, 400, c.textSecondary),
                ),
              ],
            ),
          ),
          TextBtn(
            'Copy',
            key: ValueKey('share-copy-${s.id}'),
            color: c.textSecondary,
            onTap: () => _copy(s),
          ),
          const SizedBox(width: 8),
          TextBtn(
            'Revoke',
            key: ValueKey('share-revoke-${s.id}'),
            color: c.inlineCodeText,
            onTap: () => _revoke(s),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final pad = m ? Layout.gutter : 20.0;
    final client = _client;
    final links = _links;

    if (client == null) {
      return Padding(
        padding: EdgeInsets.all(pad),
        child: Text(
          'Sign in to a hub in Sync to share pages.',
          key: const ValueKey('share-signed-out'),
          style: tsRaw(13, 18, 400, c.textSecondary),
        ),
      );
    }

    return SingleChildScrollView(
      key: const ValueKey('share-popup'),
      padding: EdgeInsets.all(pad),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text('Share', style: tsRaw(16, 24, 500, c.textPrimary)),
          const SizedBox(height: 2),
          Text(
            displayTitle(widget.pageTitle),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: tsRaw(13, 18, 400, c.textSecondary),
          ),
          const SizedBox(height: 16),
          _choice(
            context,
            group: 'visibility',
            options: const [('public', 'Public'), ('private', 'Private')],
            current: _visibility,
            onPick: (v) => setState(() => _visibility = v),
          ),
          _hint(
            context,
            _visibility == 'public'
                ? 'Anyone with the link can view.'
                : 'Only people signed in to this hub can view.',
          ),
          _choice(
            context,
            group: 'kind',
            options: const [('snapshot', 'This version'), ('live', 'Live link')],
            current: _kind,
            onPick: (v) => setState(() => _kind = v),
          ),
          _hint(
            context,
            _kind == 'live'
                ? 'Always shows the latest version.'
                : 'A copy of the page as it is now.',
          ),
          PrimaryButton(
            key: const ValueKey('share-create'),
            label: _busy ? 'Creating…' : 'Create link',
            height: 36,
            onTap: _create,
          ),
          if (_error != null) ...[
            const SizedBox(height: 8),
            Text(
              _error!,
              key: const ValueKey('share-error'),
              style: tsRaw(12, 16, 400, c.inlineCodeText),
            ),
          ],
          if (_created != null) ...[
            const SizedBox(height: 12),
            CopyableText(_created!.url, textKey: const ValueKey('share-url')),
          ],
          const SizedBox(height: 20),
          Text(
            'Links to this page',
            style: tsRaw(14, 20, 500, c.textPrimary),
          ),
          if (links == null && _error == null)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 8),
              child: Text(
                'Loading links…',
                key: const ValueKey('share-loading'),
                style: tsRaw(12, 16, 400, c.textSecondary),
              ),
            )
          else if (links != null && links.isEmpty)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 8),
              child: Text(
                'No links yet.',
                key: const ValueKey('share-empty'),
                style: tsRaw(12, 16, 400, c.textSecondary),
              ),
            )
          else if (links != null)
            for (final s in links) _row(context, s),
        ],
      ),
    );
  }
}

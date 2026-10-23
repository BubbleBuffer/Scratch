// Share a page: the "Share" popup of the page menu. It makes and manages
// the page's share links on the hub: public or private, a snapshot of the
// page or a live link. A centred panel on desktop, a sheet on phones.
// Shown only for a document page while this device is signed in to a hub.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../state/app_state.dart';
import '../../ui/ui.dart';
import '../ai/agent_token_client.dart' show AgentTokenClient;
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

/// This device's share client, or null when it is not signed in to a hub.
/// The popup, the page menu and the Shared links settings all use this.
ShareClient? shareClientOf(BuildContext context) =>
    (SharePopup.debugClientFor ?? defaultShareClient)(context);

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

/// Key for the link field made just now; a share's own id keys its row.
const _createdCopyKey = 'created';

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

  /// Which Copy button just copied (_createdCopyKey or a share id). It reads
  /// "Copied" for two seconds.
  String? _copiedKey;
  Timer? _copyTimer;

  /// The share whose row asks "Revoke this link?", waiting for a second press.
  String? _confirming;

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
        // An existing link is already in the list from the hub; only a new
        // one is added to the top.
        final links = _links;
        if (links != null && links.every((x) => x.id != share.id)) {
          _links = [share, ...links];
        }
      });
      if (_links == null) _load();
      _markCopied(_createdCopyKey);
    } on ShareException catch (e) {
      if (mounted) setState(() => _error = e.message);
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _copy(String key, String url) async {
    await Clipboard.setData(ClipboardData(text: url));
    if (mounted) _markCopied(key);
  }

  void _markCopied(String key) {
    _copyTimer?.cancel();
    setState(() => _copiedKey = key);
    _copyTimer = Timer(const Duration(seconds: 2), () {
      if (mounted) setState(() => _copiedKey = null);
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
      setState(() {
        _links = _links?.where((x) => x.id != s.id).toList();
        if (_created?.id == s.id) _created = null;
      });
    } on ShareException catch (e) {
      if (mounted) setState(() => _error = e.message);
    }
  }

  Widget _label(BuildContext context, String text, {Key? key}) => Padding(
    padding: const EdgeInsets.only(bottom: 6),
    child: Text(
      text,
      key: key,
      style: tsRaw(12, 16, 500, context.colors.textSecondary),
    ),
  );

  Widget _hint(BuildContext context, String text) => Padding(
    padding: const EdgeInsets.only(top: 6, bottom: 14),
    child: Text(
      text,
      style: tsRaw(12, 16, 400, context.colors.textSecondary),
    ),
  );

  Widget _segmented(
    BuildContext context, {
    required String group,
    required List<(String, String)> options,
    required String current,
    required ValueChanged<String> onPick,
  }) {
    final c = context.colors;
    final h = context.mobile ? 36.0 : 28.0;
    return Container(
      height: h,
      clipBehavior: Clip.antiAlias,
      decoration: BoxDecoration(
        color: c.bgPrimary,
        borderRadius: BorderRadius.circular(h / 2),
      ),
      foregroundDecoration: BoxDecoration(
        borderRadius: BorderRadius.circular(h / 2),
        border: Border.all(color: c.border),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final (value, label) in options)
            Expanded(
              child: _SegmentHalf(
                key: ValueKey('share-$group-$value'),
                label: label,
                selected: current == value,
                onTap: () => onPick(value),
              ),
            ),
        ],
      ),
    );
  }

  Widget _copyButton(BuildContext context, String key, VoidCallback onTap) =>
      TextBtn(
        _copiedKey == key ? 'Copied' : 'Copy',
        key: ValueKey(
          key == _createdCopyKey ? 'share-url-copy' : 'share-copy-$key',
        ),
        color: context.colors.textSecondary,
        onTap: onTap,
      );

  Widget _row(BuildContext context, Share s) => ShareLinkRow(
    share: s,
    copied: _copiedKey == s.id,
    confirming: _confirming == s.id,
    onCopy: () => _copy(s.id, s.url),
    onAskRevoke: () => setState(() => _confirming = s.id),
    onCancelRevoke: () => setState(() => _confirming = null),
    onRevoke: () => _revoke(s),
  );

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

    final created = _created;
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
          _label(
            context,
            'Who can view',
            key: const ValueKey('share-label-visibility'),
          ),
          _segmented(
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
          _label(
            context,
            'What to share',
            key: const ValueKey('share-label-kind'),
          ),
          _segmented(
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
          if (created != null) ...[
            const SizedBox(height: 12),
            Container(
              padding: const EdgeInsets.fromLTRB(10, 6, 4, 6),
              decoration: BoxDecoration(
                color: c.bgSecondary,
                borderRadius: Radii.field,
                border: Border.all(color: c.borderSubtle),
              ),
              child: Row(
                children: [
                  Expanded(
                    child: Text(
                      created.url,
                      key: const ValueKey('share-url'),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: ts(
                        const TypeSpec(13, 19, 400),
                        c.textPrimary,
                        mono: true,
                      ),
                    ),
                  ),
                  _copyButton(
                    context,
                    _createdCopyKey,
                    () => _copy(_createdCopyKey, created.url),
                  ),
                ],
              ),
            ),
          ],
          if (created != null && created.existing)
            Padding(
              padding: const EdgeInsets.only(top: 6),
              child: Text(
                'Link already exists',
                key: const ValueKey('share-existing'),
                style: tsRaw(12, 16, 400, c.textSecondary),
              ),
            ),
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

/// One link in a list: its kind and date, with Copy and Revoke (inline
/// confirm). The popup's list and Settings › Shared links both use it.
/// [showTitle] adds the page's title (or "Deleted page") and an Open note
/// button, which is disabled while [onOpen] is null.
class ShareLinkRow extends StatelessWidget {
  const ShareLinkRow({
    super.key,
    required this.share,
    required this.copied,
    required this.confirming,
    required this.onCopy,
    required this.onAskRevoke,
    required this.onCancelRevoke,
    required this.onRevoke,
    this.showTitle = false,
    this.onOpen,
  });

  final Share share;

  /// The Copy button reads "Copied" for a moment after a copy.
  final bool copied;

  /// The row asks "Revoke this link?" with Cancel and Revoke.
  final bool confirming;
  final VoidCallback onCopy;
  final VoidCallback onAskRevoke;
  final VoidCallback onCancelRevoke;
  final VoidCallback onRevoke;
  final bool showTitle;
  final VoidCallback? onOpen;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final s = share;
    if (confirming) {
      return Padding(
        key: ValueKey('share-row-${s.id}'),
        padding: const EdgeInsets.symmetric(vertical: 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              'Revoke this link? It stops working for everyone.',
              key: ValueKey('share-confirm-${s.id}'),
              style: tsRaw(14, 20, 400, c.textPrimary),
            ),
            const SizedBox(height: 4),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                TextBtn(
                  'Cancel',
                  key: ValueKey('share-cancel-${s.id}'),
                  color: c.textSecondary,
                  onTap: onCancelRevoke,
                ),
                const SizedBox(width: 8),
                TextBtn(
                  'Revoke',
                  key: ValueKey('share-revoke-confirm-${s.id}'),
                  color: c.inlineCodeText,
                  onTap: onRevoke,
                ),
              ],
            ),
          ],
        ),
      );
    }
    final title = s.title;
    // Phones (under 480 px) stack the actions under the text, aligned left.
    final narrow = MediaQuery.sizeOf(context).width < 480;
    final text = Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (showTitle)
          Text(
            title == null ? 'Deleted page' : displayTitle(title),
            key: ValueKey('share-title-${s.id}'),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: title == null
                ? tsRaw(14, 20, 500, c.textTertiary).copyWith(
                    fontStyle: FontStyle.italic,
                  )
                : tsRaw(14, 20, 500, c.textPrimary),
          ),
        Text(s.label, style: tsRaw(14, 20, 400, c.textPrimary)),
        Text(relativeTime(s.created), style: tsRaw(12, 16, 400, c.textSecondary)),
      ],
    );
    final actions = Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (showTitle)
          // A disabled Open note is faded, so it reads as off in both themes.
          Opacity(
            opacity: onOpen == null ? 0.4 : 1,
            child: TextBtn(
              'Open note',
              key: ValueKey('share-open-${s.id}'),
              color: c.textSecondary,
              onTap: onOpen,
            ),
          ),
        TextBtn(
          copied ? 'Copied' : 'Copy',
          key: ValueKey('share-copy-${s.id}'),
          color: c.textSecondary,
          onTap: onCopy,
        ),
        const SizedBox(width: 8),
        TextBtn(
          'Revoke',
          key: ValueKey('share-revoke-${s.id}'),
          color: c.inlineCodeText,
          onTap: onAskRevoke,
        ),
      ],
    );
    return Padding(
      key: ValueKey('share-row-${s.id}'),
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: narrow
          ? Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                text,
                const SizedBox(height: 2),
                // The buttons' own padding is 8 px; pull them back so their
                // labels line up with the text above.
                Transform.translate(offset: const Offset(-8, 0), child: actions),
              ],
            )
          : Row(
              children: [
                Expanded(child: text),
                actions,
              ],
            ),
    );
  }
}

/// One half of the two-part segmented control. The selected half is filled
/// with the accent colour and its text uses the on-accent colour (AA).
class _SegmentHalf extends StatelessWidget {
  const _SegmentHalf({
    super.key,
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Semantics(
      button: true,
      selected: selected,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: ColoredBox(
          color: selected ? c.accent : const Color(0x00000000),
          child: Center(
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: tsRaw(13, 18, 600, selected ? c.onAccent : c.textSecondary),
            ),
          ),
        ),
      ),
    );
  }
}

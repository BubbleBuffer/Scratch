// Starting and stopping sync from the settings, and at boot:
// - desktop: the embedded hub starts when enabled (the default), on the
//   configured address;
// - any device: the saved hub connection is resumed.
// Every change is saved to the settings, so the next start does the same.

import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';

import '../../core/clock.dart';
import '../../core/models.dart';
import '../../core/paths.dart';
import '../../state/sync_store.dart';
import '../../addons/agent/api.dart';
import '../ai/ai_settings.dart';
import '../ai/ai_store.dart';
import '../settings/settings_store.dart';
import 'cert_trust.dart';
import 'hub_auth.dart';

/// Called once the vault is open.
Future<void> startSyncAtBoot(SyncStore sync, AppSettings s) async {
  if (SyncStore.canHost && s.hubEnabled) {
    try {
      final at = await sync.startHub(s.hubAddress);
      AppLog.info('hub serving on $at');
    } catch (e) {
      AppLog.info('hub failed: $e');
    }
  }
  final url = s.clientUrl, token = s.clientToken;
  if (url != null && token != null) {
    try {
      await sync.connect(url, token, options: syncOptionsFor(s), trusted: trustedFor(s, url));
      AppLog.info('sync client started for $url');
    } catch (e) {
      AppLog.info('sync client failed: $e');
    }
  }
  try {
    await sync.refreshConflicts();
  } catch (_) {}
  // AI: the MCP endpoint (if enabled) and queued AI tasks.
  try {
    final ai = AiSettings.beside(s.file);
    // The agent first: AI tasks with the Agent runner use it.
    final agent = AgentStore.attach(sync.vault, ai)..useAppSettings(s);
    if (agent.enabled) await agent.ensureOpen();
    await AiStore.attach(sync.vault, sync.feed, ai).start();
  } catch (e) {
    AppLog.info('ai failed: $e');
  }
}

SyncOptions syncOptionsFor(AppSettings s) {
  final d = defaultSyncOptions();
  return SyncOptions(
    debounceMs: d.debounceMs,
    maxDelayMs: d.maxDelayMs,
    intervalMs: d.intervalMs,
    longPollMs: d.longPollMs,
    prefetchAttachments: s.prefetchAttachments,
  );
}

Future<void> setHubEnabled(SyncStore sync, AppSettings s, bool on) async {
  s.update((s) => s.hubEnabled = on);
  if (on) {
    try {
      await sync.startHub(s.hubAddress);
    } catch (_) {}
  } else {
    await sync.stopHub();
  }
}

/// Saves [addr] and restarts the hub on it if it is enabled.
Future<void> setHubAddress(SyncStore sync, AppSettings s, String addr) async {
  s.update((s) => s.hubAddress = addr);
  if (s.hubEnabled) {
    try {
      await sync.startHub(addr);
    } catch (_) {}
  }
}

Future<void> connectToHub(
  SyncStore sync,
  AppSettings s,
  String url,
  String token, {
  String? deviceLabel,
  String? account,
}) async {
  await sync.connect(url, token, options: syncOptionsFor(s), trusted: trustedFor(s, url));
  s.update((s) {
    s.clientUrl = url;
    s.clientToken = token;
    s.clientDeviceLabel = deviceLabel;
    s.clientAccount = account;
    if (deviceLabel != null) s.hubSignInUrl = url;
  });
}

Future<void> disconnectFromHub(SyncStore sync, AppSettings s) async {
  await sync.disconnect();
  s.update((s) {
    s.clientUrl = null;
    s.clientToken = null;
    s.clientDeviceLabel = null;
    s.clientAccount = null;
  });
}

/// Uses the same vault identity and connection as LAN pairing.
Future<void> signInToHub(
  SyncStore sync,
  AppSettings s,
  HubAuthService auth,
  String url,
) async {
  final result = await auth.signIn(
    hubUrl: url,
    deviceId: sync.vault.deviceId,
    deviceName: defaultTargetPlatform == TargetPlatform.android
        ? 'Android'
        : Platform.localHostname,
  );
  await connectToHub(
    sync,
    s,
    result.hubUrl,
    result.token,
    deviceLabel: result.deviceName,
    account: result.account,
  );
  if (hubAgentPlatform) {
    final agent = AgentStore.existing(sync.vault);
    agent?.useAppSettings(s);
    if (agent != null) unawaited(agent.ensureOpen());
  }
}

/// Revokes the hub token and always removes the local connection, including
/// when the hub is unreachable. Attached AI settings also fall back locally.
Future<void> signOutFromHub(
  SyncStore sync,
  AppSettings s,
  HubAuthService auth,
) async {
  final url = s.clientUrl, token = s.clientToken;
  try {
    if (url != null && token != null) {
      await auth.signOut(hubUrl: url, token: token);
    }
  } finally {
    s.update((s) {
      s.clientUrl = null;
      s.clientToken = null;
      s.clientDeviceLabel = null;
      s.clientAccount = null;
    });
    final ai =
        AgentStore.existing(sync.vault)?.settings ??
        AiStore.settingsOf(sync.vault) ??
        AiSettings.beside(s.file);
    if (ai.agentHost == AgentHost.hub) {
      ai.update((s) => s.agentHost = AgentHost.local);
    }
    await sync.disconnect();
  }
}

/// Changes background prefetching; restarts the client to apply it.
Future<void> setPrefetch(SyncStore sync, AppSettings s, bool on) async {
  s.update((s) => s.prefetchAttachments = on);
  final url = s.clientUrl, token = s.clientToken;
  if (sync.connected && url != null && token != null) {
    await sync.connect(url, token, options: syncOptionsFor(s), trusted: trustedFor(s, url));
  }
}

/// The certificate fingerprints this device trusts for [url]'s origin.
List<String> trustedFor(AppSettings s, String url) => s.trustedCertificates.fingerprintsFor(url);

/// Trusts [chain] (leaf first; [fingerprint] is the leaf's) for [url]'s
/// origin, replacing an earlier pin, and reconnects the current client if
/// it syncs with that origin.
Future<void> trustCertificate(
  SyncStore sync,
  AppSettings s,
  String url,
  List<CertDetails> chain, {
  String? fingerprint,
}) async {
  final pin = PinnedCertificate.of(chain, fingerprint: fingerprint, trustedAtMs: clockNow().millisecondsSinceEpoch);
  s.update((s) => s.trustedCertificates.add(url, pin));
  await _reconnectFor(sync, s, url);
}

/// Stops trusting the certificate for [url]'s origin, and reconnects the
/// current client if it syncs with that origin (its sync then fails until
/// a certificate is trusted again).
Future<void> revokeCertificate(SyncStore sync, AppSettings s, String url) async {
  s.update((s) => s.trustedCertificates.revoke(url));
  await _reconnectFor(sync, s, url);
}

Future<void> _reconnectFor(SyncStore sync, AppSettings s, String url) async {
  final current = sync.hubUrl;
  if (!sync.connected || current == null || originOf(current) != originOf(url)) return;
  await sync.reconnect(trusted: trustedFor(s, current));
}

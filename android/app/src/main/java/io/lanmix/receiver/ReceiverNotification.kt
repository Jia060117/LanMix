package io.lanmix.receiver

import android.app.Notification
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.net.Uri

internal const val MEDIA_CONTROL = "io.lanmix.receiver.MEDIA_CONTROL"

internal fun mediaIntent(context: Context, target: RemoteMediaTarget, action: RemoteMediaAction): Intent =
    Intent(context, ReceiverService::class.java).setAction(MEDIA_CONTROL)
        // Different sessions get different PendingIntents. An old notification must
        // never acquire the identity of a newly connected computer via UPDATE_CURRENT.
        .setData(Uri.Builder().scheme("lanmix").authority("media").appendPath(target.id)
            .appendPath(target.revision.toString()).appendPath(action.wire).build())
        .putExtra("target", target.id).putExtra("revision", target.revision)
        .putExtra("stream", target.streamId).putExtra("command", action.wire)

internal fun dispatchMediaIntent(intent: Intent, target: RemoteMediaTarget?,
    send: (RemoteMediaTarget, RemoteMediaAction) -> Boolean): Boolean {
    if (intent.action != MEDIA_CONTROL || target == null || intent.getStringExtra("target") != target.id
        || intent.getLongExtra("revision", -1) != target.revision
        || intent.getLongExtra("stream", -1) != target.streamId) return false
    val action = RemoteMediaAction.fromWire(intent.getStringExtra("command")) ?: return false
    return send(target, action)
}

/** MediaStyle supplies three compact buttons without claiming a phone media session/focus. */
internal fun receiverNotification(context: Context, channel: String, message: String,
    target: RemoteMediaTarget?): Notification {
    val flags = PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
    val content = PendingIntent.getActivity(context, 0,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP), flags)
    val stop = PendingIntent.getService(context, 1,
        Intent(context, ReceiverService::class.java).setAction(ReceiverService.STOP), flags)
    val builder = Notification.Builder(context, channel).setSmallIcon(R.drawable.ic_receiver)
        .setContentTitle(target?.let { "LanMix · ${it.name}" } ?: context.getString(R.string.receiver_notification_title))
        .setContentText(if (target == null) message else context.getString(R.string.receiver_notification_remote))
        .setContentIntent(content).setOngoing(true).setOnlyAlertOnce(true).setShowWhen(false)
        .setCategory(if (target == null) Notification.CATEGORY_SERVICE else Notification.CATEGORY_TRANSPORT)
        .setVisibility(Notification.VISIBILITY_PUBLIC)
    if (target != null) {
        for (action in RemoteMediaAction.entries) {
            val pending = PendingIntent.getService(context, 100 + action.ordinal, mediaIntent(context, target, action), flags)
            builder.addAction(Notification.Action.Builder(Icon.createWithResource(context, action.icon), action.label, pending).build())
        }
        builder.setStyle(Notification.MediaStyle().setShowActionsInCompactView(0, 1, 2))
    }
    // Keep stop-receiving available in the expanded notification.
    return builder.addAction(Notification.Action.Builder(null, context.getString(R.string.receiver_stop), stop).build()).build()
}

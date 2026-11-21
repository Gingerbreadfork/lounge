package tv.lounge.feature.home

import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Text
import tv.lounge.core.QrCode
import tv.lounge.design.LoungeTheme

/**
 * An address in groups of four over two even lines, so it can be checked by eye against what a
 * wallet shows.
 */
internal fun spaced(address: String): String {
    val groups = address.chunked(4)
    val first = (groups.size + 1) / 2
    return listOf(groups.take(first), groups.drop(first)).filter { it.isNotEmpty() }.joinToString("\n") { it.joinToString(" ") }
}

/** A coin's way to give: its code carries the address under the coin's own scheme, which wallets read. */
private fun coin(name: String, scheme: String, address: String) =
    CodeChoice(name, "$scheme:$address", spaced(address), "Scan with a wallet.\nSend only $name to this address.")

/**
 * The ways to give to Lounge's maker, the one most people can use first. Each address is held
 * to its checksum by a test.
 */
internal val Gifts = listOf(
    CodeChoice("PayPal", "https://paypal.me/gingerbreadfork", "paypal.me/gingerbreadfork", "Scan with your phone's camera.\nYou choose the amount."),
    coin("Bitcoin", "bitcoin", "bc1qlezapstmtq6jm4cj6ahn0ylyyfnpz79l2zegjv"),
    coin("DigiByte", "digibyte", "dgb1qrf5m26fjs57rs4lehdm95ycz5h8py94n3hcw2h"),
    coin("Bitcoin Cash", "bitcoincash", "qqn2ymq3qwp7987cjlllgyp56dk3m7t89vu8u48sxk"),
)

/**
 * How long Lounge has been on this TV, as the page says it: "3 weeks", "8 months", "over a
 * year". Nothing for less than a week, when it has not yet had the chance to earn anything.
 */
internal fun hadFor(sinceMs: Long?, nowMs: Long): String? {
    val days = ((nowMs - (sinceMs ?: return null)) / 86_400_000L).toInt()
    return when {
        days < 7 -> null
        days < 60 -> "${days / 7} weeks".replace("1 weeks", "1 week")
        days < 365 -> "${days / 30} months"
        days < 730 -> "over a year"
        else -> "over ${days / 365} years"
    }
}

/** The page's ask, with how long this TV has had Lounge where that is known. */
internal fun donateAsk(had: String?): String =
    (if (had != null) "You've had Lounge on this TV for $had. " else "") +
        "If it has earned its place, give what it has been worth to you. Even the price of a coffee keeps it going, and absolutely anything helps."

/**
 * The page for giving: why, under the mark and its lamp, and a code for each way to give
 * (DESIGN.md §5.5). Left and Right change the way, and Back returns to where it was opened from.
 */
@Composable
internal fun DonateScreen(
    /** Works out the code for some text, or nothing when it cannot. Called off the main thread. */
    qr: (String) -> QrCode?,
    /** When Lounge was first put on this TV, in milliseconds, if Android will say. */
    installedMs: Long?,
    visibleState: MutableTransitionState<Boolean>,
    onClose: () -> Unit,
) {
    val colors = LoungeTheme.colors
    val motion = LoungeTheme.motion
    val type = LoungeTheme.typography
    val glowAlpha = LoungeTheme.visuals.glowAlpha
    val focus = remember { FocusRequester() }
    val had = remember(installedMs) { hadFor(installedMs, System.currentTimeMillis()) }
    var up by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { up = true }
    val lamp = animateFloatAsState(if (up) 1f else 0f, motion.lamp(), label = "donateLamp")

    CodePage("Support Lounge", visibleState, onClose) {
        Column(Modifier.width(940.dp).fillMaxHeight(), verticalArrangement = Arrangement.SpaceBetween) {
            Column {
                Spacer(Modifier.height(150.dp))
                Lockup(lamp)
                Spacer(Modifier.height(56.dp))
                // What Lounge never took comes first, then the one thing it asks, then how little that is.
                Column(Modifier.width(820.dp).arrives(arriving(up, 1)), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    Text("No adverts. No tracking. No price.", style = type.displayL, color = colors.limelight)
                    Text("This page is the only place Lounge asks for anything.", style = type.headline, color = colors.houselight)
                }
                Spacer(Modifier.height(28.dp))
                Column(Modifier.width(760.dp).arrives(arriving(up, 2)), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    Text(donateAsk(had), style = type.body, color = colors.limelight)
                    Text(
                        "It takes about a minute from your phone, and all of it goes straight to its maker. Lounge stays just the same whether you give or not.",
                        style = type.meta,
                        color = colors.dust,
                    )
                }
            }
            MakerLockup("Thank you, from", Modifier.arrives(arriving(up, 4)))
        }
        Box(Modifier.fillMaxSize()) {
            CodeColumn(
                qr = qr,
                choices = Gifts,
                above = "Point your phone's camera here",
                rowTitle = "Give with",
                describe = { "A code for giving with ${it.name}, to ${it.shown}" },
                glow = { glowAlpha * 0.6f * lamp.value },
                focus = focus,
                modifier = Modifier.align(Alignment.Center).arrives(arriving(up, 3)),
            )
        }
    }
    LaunchedEffect(Unit) { focus.requestWhenReady() }
}

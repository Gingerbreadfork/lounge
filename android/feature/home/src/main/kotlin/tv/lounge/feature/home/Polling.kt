package tv.lounge.feature.home

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.CoroutineScope

/**
 * Runs [block] while Lounge is on screen and stops it while another app is in front, starting it
 * again on return; a `delay` loop in a plain effect would go on polling behind a film.
 */
@Composable
fun WhileStarted(vararg keys: Any?, block: suspend CoroutineScope.() -> Unit) {
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    LaunchedEffect(lifecycle, *keys) { lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED, block) }
}

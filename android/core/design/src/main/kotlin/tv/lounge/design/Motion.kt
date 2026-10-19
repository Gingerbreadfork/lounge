package tv.lounge.design

import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.tween
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf

/** Curves and durations (DESIGN.md §6.2). No overshoot, no bounce. */
@Immutable
data class LoungeMotion(
    val snapMs: Int,
    val settleMs: Int,
    val travelMs: Int,
    val prosceniumMs: Int,
    val curtainMs: Int,
    /** The house lights going down when nobody has touched the remote for a while. */
    val dimMs: Int,
    /** The lamp warming up after a cold start. */
    val lampMs: Int,
    val reducedMotion: Boolean,
) {
    val snapEasing: Easing = CubicBezierEasing(0.2f, 0f, 0f, 1f)
    val settleEasing: Easing = CubicBezierEasing(0.2f, 0f, 0f, 1f)
    val travelEasing: Easing = CubicBezierEasing(0.3f, 0f, 0.1f, 1f)
    val prosceniumEasing: Easing = CubicBezierEasing(0.35f, 0f, 0.15f, 1f)
    val curtainEasing: Easing = CubicBezierEasing(0.3f, 0f, 0.1f, 1f)
    val dimEasing: Easing = CubicBezierEasing(0.4f, 0f, 0.2f, 1f)
    val lampEasing: Easing = CubicBezierEasing(0.2f, 0f, 0f, 1f)

    fun <T> snap(): FiniteAnimationSpec<T> = tween(snapMs, easing = snapEasing)
    fun <T> settle(): FiniteAnimationSpec<T> = tween(settleMs, easing = settleEasing)
    fun <T> travel(): FiniteAnimationSpec<T> = tween(travelMs, easing = travelEasing)
    fun <T> proscenium(): FiniteAnimationSpec<T> = tween(prosceniumMs, easing = prosceniumEasing)
    fun <T> curtain(): FiniteAnimationSpec<T> = tween(curtainMs, easing = curtainEasing)
    fun <T> dim(): FiniteAnimationSpec<T> = tween(dimMs, easing = dimEasing)
    fun <T> lamp(): FiniteAnimationSpec<T> = tween(lampMs, easing = lampEasing)

    companion object {
        val Default = LoungeMotion(
            snapMs = 120,
            settleMs = 180,
            travelMs = 220,
            prosceniumMs = 260,
            curtainMs = 320,
            dimMs = 1200,
            lampMs = 600,
            reducedMotion = false,
        )

        /** Everything becomes instant except travel, which stays trackable. */
        val Reduced = LoungeMotion(
            snapMs = 1,
            settleMs = 1,
            travelMs = 120,
            prosceniumMs = 1,
            curtainMs = 1,
            dimMs = 1,
            lampMs = 1,
            reducedMotion = true,
        )
    }
}

val LocalLoungeMotion = staticCompositionLocalOf { LoungeMotion.Default }

@Composable
fun motion(): LoungeMotion = LocalLoungeMotion.current

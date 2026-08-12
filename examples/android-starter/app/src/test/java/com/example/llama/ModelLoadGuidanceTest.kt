package com.example.llama

import org.junit.Assert.assertEquals
import org.junit.Test

class ModelLoadGuidanceTest {
    @Test
    fun recommendsBaselineOnlyForBackendFailures() {
        assertEquals(
            "The optimized backend could not start. Install the baseline build on this device.",
            modelLoadGuidance("[VIFU-LLAMA-BACKEND-002] no device", "optimized"),
        )
        assertEquals(
            "The selected GGUF cannot be read. Choose the model again.",
            modelLoadGuidance("[VIFU-LLAMA-MODEL-001] cannot read", "optimized"),
        )
    }

    @Test
    fun distinguishesOptimizedAndBaselineModelLoadFailures() {
        val failure = "[VIFU-LLAMA-MODEL-003] null result from llama cpp"

        assertEquals(
            "llama.cpp could not load this model with the optimized backend. Try the baseline build.",
            modelLoadGuidance(failure, "optimized"),
        )
        assertEquals(
            "llama.cpp could not load this GGUF. Check the model format and file.",
            modelLoadGuidance(failure, "baseline"),
        )
    }
}

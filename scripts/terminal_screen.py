"""Screen observation for headless Steelix checks (requires pyte)."""
import pyte


class Screen(pyte.Screen):
    # Helix probes terminal capabilities; tests only observe output.
    def report_device_status(self, *args, **kwargs):
        pass

    def report_device_attributes(self, *args, **kwargs):
        pass

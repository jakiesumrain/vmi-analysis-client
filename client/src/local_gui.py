"""Client-owned entry point for a local VMI checkout."""

import json
import os


def main():
    connection = json.loads(os.environ.pop("VMI_CLIENT_AGENT_CONNECTION", "null"))

    from vmi_analysis.gui.app import main as run_gui

    if connection is None:
        run_gui(host="127.0.0.1")
        return

    from vmi_analysis.agent_backend import AGENTS, AgentRequest
    from vmi_analysis.gui import shared
    from vmi_analysis.paths import GENERATED_DIR

    request = AgentRequest(
        owner="local",
        workspace=shared.workspace_root(),
        config_root=GENERATED_DIR / "agent",
        base_url=connection["base_url"],
        api=connection["api"],
        model=connection["model"],
        api_key=connection.pop("api_key"),
        additional_models=tuple(connection["additional_models"]),
        image_models=tuple(connection["image_models"]),
    )
    history_factory = getattr(shared, "agent_history", None)
    options = {"history": history_factory(None)} if history_factory else {}
    try:
        connect = getattr(AGENTS, "connect_provider", None) or AGENTS.start
        connect(request, shared_mode=False, **options)
    except (OSError, RuntimeError, ValueError):
        raise SystemExit(78) from None
    finally:
        del request

    try:
        run_gui(host="127.0.0.1")
    finally:
        AGENTS.shutdown()


if __name__ == "__main__":
    main()

"""Run the versioned Oh My Pi harness inside a Harbor task container."""

import json
import shlex

from harbor.agents.installed.base import BaseInstalledAgent, CliFlag, with_prompt_template
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext


class OmpAgent(BaseInstalledAgent):
    """Harbor adapter for the real OMP CLI, not upstream Pi."""

    _OUTPUT_FILENAME = "omp.jsonl"
    _DEFAULT_VERSION = "17.0.3"

    CLI_FLAGS = [
        CliFlag(
            "thinking",
            cli="--thinking",
            type="enum",
            choices=["off", "minimal", "low", "medium", "high", "xhigh", "max"],
            default="high",
        )
    ]

    @staticmethod
    def name() -> str:
        return "omp"

    def get_version_command(self) -> str | None:
        return 'export PATH="$HOME/.bun/bin:$PATH"; omp --version'

    def parse_version(self, stdout: str) -> str:
        return stdout.strip().splitlines()[-1].strip()

    async def install(self, environment: BaseEnvironment) -> None:
        await self.exec_as_root(
            environment,
            command="apt-get update && apt-get install -y --no-install-recommends ca-certificates curl unzip",
            env={"DEBIAN_FRONTEND": "noninteractive"},
        )
        version = self._version or self._DEFAULT_VERSION
        await self.exec_as_agent(
            environment,
            command=(
                "set -euo pipefail; "
                "curl -fsSL https://bun.sh/install | bash && "
                'export PATH="$HOME/.bun/bin:$PATH" && '
                f"bun add --global @oh-my-pi/pi-coding-agent@{shlex.quote(version)} && "
                "omp --version"
            ),
        )

    @with_prompt_template
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        if not self.model_name or "/" not in self.model_name:
            raise ValueError("model name must be in provider/model format")

        provider = self.model_name.split("/", 1)[0]
        credential_names = {
            "anthropic": ["ANTHROPIC_API_KEY", "ANTHROPIC_OAUTH_TOKEN"],
            "google": ["GEMINI_API_KEY", "GOOGLE_API_KEY"],
            "openai": ["OPENAI_API_KEY"],
            "openrouter": ["OPENROUTER_API_KEY"],
            "xai": ["XAI_API_KEY"],
            "zai": ["ZAI_API_KEY"],
        }.get(provider, [])
        env = {
            name: value
            for name in credential_names
            if (value := self._get_env(name))
        }
        if credential_names and not env:
            raise ValueError(f"no credential configured for provider {provider!r}")
        credential_guard = ""
        if credential_names:
            any_present = " || ".join(
                f'[ -n "${{{name}:-}}" ]' for name in credential_names
            )
            credential_guard = (
                f"{any_present} || "
                f'{{ echo "missing {provider} credential in Harbor task environment" >&2; exit 78; }}; '
            )
        credential_flag = ""
        if env:
            credential_name = next(iter(env))
            credential_flag = f'--api-key "${{{credential_name}}}" '

        flags = self.build_cli_flags()
        escaped_instruction = shlex.quote(instruction)
        escaped_provider = shlex.quote(provider)
        escaped_model = shlex.quote(self.model_name.split("/", 1)[1])
        await self.exec_as_agent(
            environment,
            command=(
                f'set -euo pipefail; export PATH="$HOME/.bun/bin:$PATH"; {credential_guard}'
                "omp --print --mode json --no-session --no-skills --no-rules "
                "--approval-mode yolo --max-time 280 "
                "--tools read,bash,edit,write,grep,glob "
                f"{credential_flag}--provider {escaped_provider} --model {escaped_model} "
                f"{flags} {escaped_instruction} "
                f"2>&1 </dev/null | stdbuf -oL tee /logs/agent/{self._OUTPUT_FILENAME}"
            ),
            env=env,
        )
        self._raise_on_model_error()

    def _raise_on_model_error(self) -> None:
        output_file = self.logs_dir / self._OUTPUT_FILENAME
        if not output_file.exists():
            raise RuntimeError("OMP exited without writing its JSONL transcript")

        saw_assistant = False
        for line in output_file.read_text().splitlines():
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if event.get("type") != "message_end":
                continue
            message = event.get("message") or {}
            if message.get("role") != "assistant":
                continue
            saw_assistant = True
            if message.get("stopReason") == "error":
                status = message.get("errorStatus")
                detail = str(message.get("errorMessage") or "provider error")[:300]
                raise RuntimeError(f"OMP model call failed (status={status}): {detail}")

        if not saw_assistant:
            raise RuntimeError("OMP transcript contains no completed assistant message")

    def populate_context_post_run(self, context: AgentContext) -> None:
        output_file = self.logs_dir / self._OUTPUT_FILENAME
        if not output_file.exists():
            return

        input_tokens = 0
        output_tokens = 0
        cached = 0
        cost_usd = 0.0
        for line in output_file.read_text().splitlines():
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if event.get("type") != "message_end":
                continue
            message = event.get("message") or {}
            if message.get("role") != "assistant":
                continue
            usage = message.get("usage") or {}
            input_tokens += usage.get("input", 0)
            output_tokens += usage.get("output", 0)
            cached += usage.get("cacheRead", 0)
            cost_usd += (usage.get("cost") or {}).get("total", 0.0)

        context.n_input_tokens = input_tokens + cached
        context.n_output_tokens = output_tokens
        context.n_cache_tokens = cached
        context.cost_usd = cost_usd if cost_usd > 0 else None

Technical Foundations of Agentic Engineering: Architectural Patterns, Contextual Sovereignty, and Cognitive Planning in Autonomous Systems
The shift in the artificial intelligence landscape from passive, chat-centric models to autonomous, goal-oriented agentic systems represents a fundamental transformation in software engineering. This transition is not merely an increase in model intelligence but a paradigm shift in how computational systems interact with data, tools, and environments. Research and engineering disclosures from Anthropic, OpenAI, and Google indicate a growing consensus that the reliability of agentic systems depends on three pillars: modular architectural patterns, sophisticated context management through progressive disclosure, and the formalization of planning as a first-class cognitive artifact. By examining the disparate approaches of these organizations, engineers can derive a set of first principles for building systems capable of long-horizon tasks and complex environment interaction.
Foundations of Agentic Architectures
At the core of agentic engineering is the decision of how to structure the relationship between the language model and the task at hand. While early implementations relied on monolithic prompts, modern systems have pivoted toward modular, multi-agent, and sub-agent architectures to overcome the inherent limitations of single-model reasoning. The architectural design of an agent system is the primary determinant of its success rate on complex, multi-step tasks.
Multi-Agent Coordination and the Alignment Principle
The evolution of agentic systems has moved from a single, "exhausted generalist" model to a "team of specialists" approach.[1] This transition is driven by the realization that as the scope of an agent’s responsibilities grows, the performance of a single-agent system often hits a ceiling.[2, 3] Multi-agent systems (MAS) mitigate this by assigning specific roles and tasks to individual agents, effectively creating a microservices architecture for artificial intelligence.[1, 4]
Google Research has derived quantitative scaling principles for these systems, identifying that the effectiveness of multi-agent coordination is strictly dependent on the properties of the task, specifically its degree of parallelizability versus its sequentiality.[3] This discovery, known as the Alignment Principle, provides a critical heuristic for architectural selection. On parallelizable tasks, such as complex financial reasoning where distinct agents can simultaneously analyze revenue trends, cost structures, and market comparisons, centralized coordination can improve performance by as much as 80.9% over a single-agent baseline.[3] The ability to decompose a complex problem into independent sub-tasks allows for a "validation bottleneck" where a central orchestrator catches errors before they propagate through the system.[3]
Conversely, tasks requiring strict sequential reasoning, where each step depends on the precise outcome of the previous one, suffer from a "sequential penalty" when distributed across multiple agents. In such scenarios, multi-agent variants often degrade performance by 39% to 70%.[3] This degradation occurs because the overhead of inter-agent communication fragments the reasoning process, consuming the "cognitive budget" that would otherwise be spent on task execution.[3]
Multi-Agent Interaction Patterns
The choice of design pattern offers a framework for organizing system components and orchestrating the workflow. Google and Anthropic have documented several distinct patterns for multi-agent systems, each with unique implementation logic and trade-offs.
Pattern
Implementation Logic
Ideal Application
Coordinator
A single "parent" agent manages specialized sub-agents, delegating tasks and synthesizing outputs. [5]
Open-ended research, multi-source information gathering.
Sequential
A rigid, deterministic workflow where the output of one agent is the direct input of the next. [5]
Fixed-step data processing, automated legal document review.
Parallel
Multiple specialized sub-agents perform independent tasks simultaneously, with results merged at the end. [5]
Vulnerability scanning, simultaneous multi-market analysis.
Swarm
A collaborative, all-to-all communication model where agents communicate dynamically to refine solutions. [5]
Highly ambiguous tasks requiring creative synthesis.
Evaluator-Optimizer
One agent generates a response while another critiques and improves it in a recurring loop. [6, 7]
Code optimization, high-quality content generation.
Hierarchical Decomposition
A multi-level hierarchy where a root agent decomposes tasks into layers of sub-tasks for worker agents. [5]
Large-scale software development, complex project planning.
The Swarm Framework and Handoff Primitives
OpenAI’s Swarm framework represents an educational exploration into lightweight, ergonomic multi-agent orchestration.[8] The Swarm architecture is built on two primary abstractions: Agents and Handoffs. An Agent in this context encompasses a specific set of instructions and functions, effectively representing a persona or a specific step in a complex retrieval process.[8]
Coordination is achieved through the "handoff" mechanism, which occurs when one agent decides to transfer the conversation to another.[8] This is triggered when an agent’s function returns another Agent object. When the execution client detects this return, it switches the active agent, replacing the old system prompt with the new agent’s instructions while maintaining the conversation history.[8] This ensures continuity while allowing for a specialized shift in reasoning. This "handoff" approach is particularly suited for situations dealing with a large number of independent capabilities that are difficult to encode into a single monolithic prompt.[8]
Advanced Task Planning and "Plan Mode" Engineering
The introduction of reasoning models, such as the OpenAI o-series and Anthropic’s extended thinking capabilities, has shifted the agentic engineering focus from simple execution to sophisticated internal planning. This transition is characterized by models that "think before they answer," producing an internal chain of thought (CoT) that allows them to refine their approach before generating user-facing output.[9, 10, 11]
Internal Reasoning and Chain of Thought
Reasoning models like OpenAI o1 use reinforcement learning to develop their reasoning ability, learning to break down problems, evaluate multiple solution paths, and backtrack when an approach is not working.[9, 12] This process generates "reasoning tokens"—internal computations that are not typically visible in the final completion but occupy space in the context window.[11]
For agentic workflows, this means that the model acts less like a junior coworker needing micro-instructions and more like a senior colleague who can be trusted to work out the details of a high-level goal.[11, 13] This internal planning is highly effective for complex problems in coding, advanced mathematics, and scientific research, where multi-step logic is critical for accuracy.[9, 10, 12]
Controlling the Planning Depth
Developers can guide the planning process through specific parameters. OpenAI’s reasoning.effort parameter, for instance, allows for low, medium, or high effort levels, guiding the model on how many reasoning tokens to generate before responding.[11] A "high" effort setting favors complete reasoning for complex tasks, while "low" effort settings favor speed and token economy.[11]
When integrating these models into agentic loops, it is recommended to pass back all "reasoning items" from previous calls.[11] This allows the model to continue its reasoning process across multiple tool calls more efficiently, avoiding the need to restart its internal "thinking" from scratch for every turn.[11] This stateful management of reasoning context is a cornerstone of modern agentic engineering, as it ensures that the model maintains its cognitive momentum across multi-turn interactions.
Planning as a First-Class Artifact
In addition to internal reasoning, agentic systems increasingly treat plans as explicit, versioned artifacts. OpenAI’s "Harness Engineering" best practices suggest that for small changes, ephemeral lightweight plans suffice, but for complex work, "execution plans" should be generated.[14] These plans, which include progress and decision logs, should be checked into the repository alongside the code. By co-locating active plans, completed plans, and technical debt, agents can operate without relying on external, unreachable context.[14]
This approach of "pulling the system into a form the agent can inspect" increases the leverage of the agent, allowing it to validate and modify its own strategy based on the environment state.[14] It also provides a transparent audit trail for human supervisors, who can inspect the execution plan to understand the agent’s intended trajectory and intervene if the plan deviates from the desired outcome.[7, 14]
Memory Systems and State Management
A primary challenge for long-running agents is the accumulation of history, which leads to "context rot"—a state where a giant instruction file or a long history of actions and observations crowds out the relevant task data, causing the agent to lose focus or miss key constraints.[14, 15] Memory systems are designed to manage this through structured storage, selective retrieval, and context compression.
Long Context Paradigms: Google Gemini
Google’s Gemini models represent a shift in memory management by offering massive context windows of 1 million to 2 million tokens.[16, 17] In this paradigm, "short-term memory" is effectively expanded to include entire codebases, long document corpuses, or hours of video.[17, 18] This enables "many-shot in-context learning," where a model is presented with hundreds or thousands of examples rather than just a few, allowing it to learn new tasks—such as translating a rare language—directly from instructional material provided in-context.[16, 17]
However, even with millions of tokens, performance is not infinite. Retrieval accuracy, while near-perfect (>99%) for single pieces of information ("needles"), can decrease when the model must retrieve multiple specific items simultaneously.[16, 17] In these cases, developers must still rely on architectural patterns or multiple requests to ensure high performance.
Context Caching and Economic Efficiency
The primary optimization for long-context systems is context caching. Traditionally, a "chat with your data" application required moving data into the context window for every request, which was both expensive and latent.[16] Context caching allows developers to cache large datasets—such as a library of PDFs or a video file—and pay a per-hour storage fee.[16, 17] This reduces the cost of subsequent requests by a factor of four or more while maintaining high performance.[16]
Structured Artifacts for Persistence
For agents working across multiple context windows, Anthropic suggests using external persistent storage to maintain state. The core challenge of long-running agents is that each new session begins with a fresh context window and no memory of what came before.[19] To solve this, agents use specific "memory artifacts":
claude-progress.txt: This file serves as a summary of the work done so far. At the end of every session, the agent writes a progress report to this file, which the next agent instance reads to get its bearings.[19]
feature_list.json: This document tracks the status of various features being implemented. Each feature starts as "failing" and is only marked as "passing" after the agent runs verification tests. Using JSON instead of Markdown prevents the agent from accidentally overwriting the entire requirement list.[19]
Git History: By utilizing standard development tools like git, agents can track file changes and use commit messages to understand the recent logic and recover from broken states.[19]
init.sh: Initializer agents create setup scripts to define how to run the development environment, ensuring that subsequent agents can restart the server and run tests without manual configuration.[19]
This system of "harnessing" agents creates a software project staffed by "engineers working in shifts," where the documentation serves as the handover mechanism between context windows.[19]
Context Management and Progressive Disclosure
As agentic systems incorporate more tools and capabilities, managing what information enters the context window becomes an architectural necessity. Progressive disclosure is a technique derived from UI design that shows only the necessary information at each moment, hiding complexity until it is required.[20, 21]
The Agent Skills Architecture
Anthropic’s "Agent Skills" system implements progressive disclosure to control context and token usage. Rather than loading all available capabilities into the context from the start, the system organizes tools into layers of detail [15, 20, 21]:
Discovery (Layer 1): The agent sees only lightweight metadata, such as the skill name and a short description. This allows the agent to identify relevant capabilities without saturating the context window.[15, 21]
Activation (Layer 2): If the agent determines a skill is useful for the task, the system then loads the specific instructions required to use that skill correctly.[21]
Execution (Layer 3): Detailed examples, extensive documentation, or reference material are added only when the agent is actually executing the skill.[21]
This hierarchical approach improves tool selection accuracy by reducing the number of simultaneous instructions the model must consider.[21] It also enables modular and scalable architectures, where new skills can be added to the system without inflating the base context.[21]
Recursive Context Assembly
Progressive disclosure can also be applied recursively to information retrieval. In this pattern, the agent starts with a high-level overview of an account or a database, which reveals specific areas of interest (e.g., open support tickets or specific files).[22] The agent then makes subsequent calls to "drill down" into these specific areas, gathering context hierarchically based on what the previous layer surfaced.[22]
This "Just-in-Time" (JIT) context strategy ensures that the agent only ever loads what is specifically needed for the current request. While this approach may increase latency due to multiple inference calls, it nets out in precision and superior context management, allowing agents to navigate massive digital environments without becoming overwhelmed.[15, 22]
Context Compression and ACON
For long-chain tasks that exceed the available context, systems utilize context compression. The Agent Context Optimization (ACON) framework provides a unified system for compressing environment observations and interaction histories.[23] ACON does not simply summarize text; it identifies task-relevant signals—such as action-outcome relationships and API formats—and discards extraneous data.[23]
A key innovation in ACON is "gradient-free guideline optimization." The system uses a capable LLM to analyze "paired trajectories"—one where a full context led to a successful task and one where a compressed context caused a failure.[23] The LLM identifies the causes of failure (e.g., a missing identifier) and updates the compression guidelines to ensure that similar critical information is preserved in the future.[23] This distilled compression logic can then be run on smaller, more efficient models, reducing memory usage by 26% to 54% while preserving task performance.[23]
Tool Calling and the Agent-Computer Interface (ACI)
Agents interact with their environment through tools, which define the contract between the agent and its information space. Developing effective tools requires focusing on "ergonomics"—how intuitive and efficient the tools are for the language model.[24, 25]
Model Context Protocol (MCP)
The Model Context Protocol (MCP) is an open standard designed to improve agentic workflows by providing a universal standard for tool and data integration.[20, 26, 27] MCP allows agents to connect to potentially hundreds of tools while maintaining control over the tooling surface. By registering vetted MCP tool servers, developers can ensure that agents have access to the exact resources they need without manual prompt engineering for every new tool.[28]
Computer Use and Graphical Interaction
A significant breakthrough in agentic engineering is the ability of models to interact with graphical user interfaces (GUIs) directly. OpenAI’s "Operator" and Anthropic’s "Computer Use" capabilities allow agents to perceive the screen as raw pixel data and take actions using a virtual mouse and keyboard.[29, 30, 31]
The standard "Computer Use" loop follows a structured cycle of perception, reasoning, and action:
Perception: The agent captures screenshots of the display to analyze the current state of the digital environment.[29, 30, 32]
Reasoning: Using chain-of-thought reasoning, the agent evaluates the next steps by considering both current and past screenshots and actions.[30, 31]
Action: The agent executes tasks like clicking, scrolling, or typing until the task is complete or user input is needed.[30, 31]
These "Computer-Using Agents" (CUA) can navigate buttons, menus, and text fields just as humans do, allowing them to perform digital tasks without requiring specialized web or OS APIs.[31, 33] Benchmarks like OSWorld and WebArena show that CUA models are setting new state-of-the-art results, although they still face challenges with flakiness on practical automations, such as layout changes or window focus loss.[31, 34]
Best Practices for Tool Engineering
Research suggests several key principles for writing tools that are effective for agents:
Non-Overlapping Functionality: Tools should be purpose-specific and self-contained to avoid confusing the model during selection.[15, 24]
Gerund-Style Naming: Descriptive names like processing-pdfs are more effective than vague names like helper.[15]
Poka-Yoke Your Tools: Change arguments so that it is harder to make mistakes (e.g., using enums for restricted inputs).[7]
Verbose Docstrings: Tool descriptions should include examples of good and bad usage, edge cases, and clear boundaries from other tools.[7]
Third-Person Descriptions: Anthropic mandates writing tool descriptions in the third person (e.g., "Processes Excel files") to prevent inconsistent points of view in the system prompt.[35]
Handling Long-Chain Tasks and the Forgetting Problem
The "forgetting problem" in long-chain tasks is often a result of context saturation or the propagation of errors across multiple steps. To combat this, advanced frameworks like MAKER employ mechanisms to ensure long-horizon reliability.[36]
Maximal Agentic Decomposition (MAD)
The MAKER framework utilizes Maximal Agentic Decomposition (MAD), where a task is divided into the smallest possible sub-problems—often a single decision per agent.[36] Each agent receives only the minimal context needed for its assigned step. This modularity prevents context drift, isolates errors, and enables efficient error correction.[36]
Voting and Consensus Mechanisms
To ensure reliability across millions of dependent steps, the MAKER framework employs "First-to-ahead-by-k-voting".[36] Several agents attempt the same step in parallel, and the system accepts the first action to achieve k more votes than any other.[36] This local consensus mechanism allows small gains in absolute accuracy per step to compound exponentially, turning localized agreement into global reliability.[36]
Recursive Summarization and Memory Updating
Recursive summarization enables long-term dialogue memory by condensing history into higher-level representations.[37] Unlike retrieval-augmented systems that depend on single-granularity segments, recursive summarization creates a multi-layered memory where essential historical information is preserved.[37]
For long-running tasks, agents should periodically summarize intermediate steps and reset the context with this summary. This "compaction" keeps key information like "User wants X, tried Y, learned Z" while discarding the full conversation transcript.[15] Reactive compression is used in systems like MassGen, where the system reacts to context overflow by generating a summary of work done and automatically retrying the request.[38] This ensures that tool calls, results, and reasoning are preserved in the summary, allowing the model to naturally continue from its own summary rather than starting fresh.[38]
Deployment Guidance and Engineering Comparison
Choosing between the primary agentic platforms—Anthropic Claude, OpenAI AgentKit, and Google Vertex AI—depends on the specific engineering priorities of the team.
Comparative Analysis of Agent Platforms
Feature
Anthropic Claude SDK
OpenAI AgentKit
Google Vertex AI Agent Builder
Philosophy
SDK-first, decentralized, developer-centric. [28]
Product-first, centralized, velocity-centric. [28]
Enterprise-first, governed, ecosystem-centric. [27, 39]
Tooling
Explicit MCP tool servers, registered and typed. [28]
Integrated "Operator" and custom browser automation. [33, 40]
100+ enterprise connectors and GCP-native control plane. [27, 41]
Context
Emphasizes economics via caching and JIT context. [15]
Emphasizes stateful reasoning via Responses API. [11]
Emphasizes massive 2M token windows. [16, 17]
Safety
Human-in-the-loop, permission boundaries, Artifacts. [34, 35]
Built-in guardrails, "Watch mode," and Operator System Card. [33]
Centralized IT management and audit trails. [34, 41]
Best For
On-prem execution, strict compliance, infra-heavy teams. [28]
Consumer-facing apps, rapid prototyping, multimodal tasks. [28, 42]
Governed enterprise scale, long document analysis. [34]
Practical Deployment Strategies
Technical teams should focus on "locking the runner before the model"—that is, building a state-aware, execution-based harness that is independent of the underlying LLM.[34] This allows for the hot-swapping of models as capabilities evolve.
For research and complex document analysis, Anthropic's long-context economics and Claude's workflow ergonomics often prove superior.[42] For applications requiring multi-tool orchestration and a single programmable substrate, OpenAI's Responses API and AgentKit stack provide a more coherent experience.[28, 34] Google’s ecosystem remains the standard for enterprises requiring governed deployment with wide surface integration and massive multimodal perception.[34]
Conclusions and Future Outlook
Agentic engineering is moving toward a standard of "deterministic reliability" through layered permission architectures and model-agnostic standards.[35] The founding of the Agentic AI Foundation signals a future where agents from different providers will negotiate and communicate through open protocols like MCP and A2A to deliver value.[27, 35]
The core lesson for engineers is to maintain simplicity in agent design and prioritize transparency by explicitly showing the agent’s planning steps.[7] By treating planning as an artifact, utilizing progressive disclosure to manage context, and employing consensus mechanisms like voting for long-horizon tasks, developers can build agentic systems that are more efficient, scalable, and reliable than those built on simple prompt-response patterns. The future of AI is not just a more intelligent model, but a more effectively engineered agent system.
--------------------------------------------------------------------------------
OpenAI Swarm Framework Guide for Reliable Multi-Agents - Galileo AI, https://galileo.ai/blog/openai-swarm-framework-multi-agents
Blog: Understanding OpenAI Swarm: A Framework for Multi-Agent Systems - Lablab.ai, https://lablab.ai/blog/understanding-openai-swarm-a-framework-for-multi-agent-systems
Towards a science of scaling agent systems: When and why agent ..., https://research.google/blog/towards-a-science-of-scaling-agent-systems-when-and-why-agent-systems-work/
OpenAI Swarm: A Hands-On Guide to Multi-Agent Systems - Analytics Vidhya, https://www.analyticsvidhya.com/blog/2024/12/managing-multi-agent-systems-with-openai-swarm/
Choose a design pattern for your agentic AI system | Cloud ..., https://docs.cloud.google.com/architecture/choose-design-pattern-agentic-ai-system
Building Effective AI Agents - Anthropic, https://resources.anthropic.com/building-effective-ai-agents
Building Effective AI Agents - Anthropic, https://www.anthropic.com/research/building-effective-agents
openai/swarm: Educational framework exploring ergonomic ... - GitHub, https://github.com/openai/swarm
What is OpenAI's o1 Model and When to Use It - MindStudio, https://www.mindstudio.ai/blog/openai-o1/
Learning to reason with LLMs | OpenAI, https://openai.com/index/learning-to-reason-with-llms/
Reasoning models | OpenAI API - OpenAI for developers, https://developers.openai.com/api/docs/guides/reasoning/
OpenAI o1 Models Make it Easier to build More Intelligent AI Agents - Unvired, https://unvired.com/blog/openai-o1-models-make-it-easier-to-build-more-intelligent-ai-agents/
Reasoning best practices | OpenAI API, https://developers.openai.com/api/docs/guides/reasoning-best-practices/
Harness engineering: leveraging Codex in an agent-first world ..., https://openai.com/index/harness-engineering/
Claude's Context Engineering Secrets: Best Practices Learned from Anthropic | Bojie Li, https://01.me/en/2025/12/context-engineering-from-claude/
Long context | Gemini API | Google AI for Developers, https://ai.google.dev/gemini-api/docs/long-context
Long context | Generative AI on Vertex AI - Google Cloud Documentation, https://docs.cloud.google.com/vertex-ai/generative-ai/docs/long-context
Introducing Gemini 1.5, Google's next-generation AI model - The Keyword, https://blog.google/innovation-and-ai/products/google-gemini-next-generation-model-february-2024/
Effective harnesses for long-running agents \ Anthropic, https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents
Anthropic Announces Claude CoWork - InfoQ, https://www.infoq.com/news/2026/01/claude-cowork/
Progressive Disclosure: the technique that helps control context (and tokens) in AI agents, https://medium.com/@martia_es/progressive-disclosure-the-technique-that-helps-control-context-and-tokens-in-ai-agents-8d6108b09289
Progressive disclosure, applied recursively; is this, theoretically, the key to infinite context?, https://www.reddit.com/r/AI_Agents/comments/1rklfzt/progressive_disclosure_applied_recursively_is/
(PDF) ACON: Optimizing Context Compression for Long-horizon ..., https://www.researchgate.net/publication/396094104_ACON_Optimizing_Context_Compression_for_Long-horizon_LLM_Agents
Effective context engineering for AI agents \ Anthropic, https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
Writing effective tools for AI agents—using AI agents - Anthropic, https://www.anthropic.com/engineering/writing-tools-for-agents
Anthropic Academy: Claude API Development Guide, https://www.anthropic.com/learn/build-with-claude
Vertex AI Agent Builder | Google Cloud, https://cloud.google.com/products/agent-builder
OpenAI AgentKit vs Claude Agents SDK: Which is better? - Bind AI, https://blog.getbind.co/openai-agentkit-vs-claude-agents-sdk-which-is-better/
Computer use tool - Claude API Docs, https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool
OpenAI Operator - Cobus Greyling - Medium, https://cobusgreyling.medium.com/openai-operator-845ee152aed0
Computer-Using Agent - OpenAI, https://openai.com/index/computer-using-agent/
Computer-using agent (CUA) models - ZBrain, https://zbrain.ai/cua-models/
Introducing Operator - OpenAI, https://openai.com/index/introducing-operator/
Google vs OpenAI vs Anthropic: The Agentic AI Arms Race Breakdown - MarkTechPost, https://www.marktechpost.com/2025/10/25/google-vs-openai-vs-anthropic-the-agentic-ai-arms-race-breakdown/
OpenAI vs Anthropic: divergent philosophies in AI Skills architecture | by Tao An | Medium, https://tao-hpu.medium.com/openai-vs-anthropic-divergent-philosophies-in-ai-skills-architecture-40a151e0f54e
Shattering the Illusion: MAKER Achieves Million-Step, Zero-Error LLM Reasoning, https://www.cognizant.com/us/en/ai-lab/blog/maker
Recursively summarizing enables long-term dialogue memory in large language models | Request PDF - ResearchGate, https://www.researchgate.net/publication/390703800_Recursively_summarizing_enables_long-term_dialogue_memory_in_large_language_models
Memory and Context Management — MassGen 0.1.0 documentation, https://docs.massgen.ai/en/latest/user_guide/sessions/memory.html
Claude vs OpenAI Agents: A Deep Dive Analysis - Sparkco, https://sparkco.ai/blog/claude-vs-openai-agents-a-deep-dive-analysis
Open AI Operator: Complete and Updated Guide 2025 - NoCode Startup, https://nocodestartup.io/en/open-ai-operator/
Vertex AI Agent Builder overview | Google Cloud Documentation, https://docs.cloud.google.com/agent-builder/overview
Anthropic vs OpenAI - Lil Big Things, https://www.lilbigthings.com/post/anthropic-vs-openai
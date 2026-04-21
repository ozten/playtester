//! Agent trait and built-in agents (`RandomAgent`, `ScriptedAgent`).
//!
//! Agents choose one action from the engine's enumerated legal actions. They
//! never mutate game state or adjudicate rules — the engine is authoritative.

# Optimi-ez

Optimizador de Windows para juegos, escrito en Rust.

> **Estado: inicio del proyecto.** Este repositorio todavía no contiene código que cambie servicios ni ajustes de Windows. No lo uses para aplicar optimizaciones.

## Objetivo

Crear perfiles por juego que identifiquen los servicios de Windows con evidencia de que afectan negativamente la experiencia gamer y permitan deshabilitarlos de forma transparente, segura y reversible. Que un servicio esté activo no significa por sí solo que perjudique el rendimiento: cada recomendación deberá explicar su evidencia y validarse con mediciones.

## Principios de seguridad

- Mostrar cada cambio propuesto, sus posibles efectos y riesgos; pedir confirmación explícita antes de aplicarlo.
- No desactivar servicios críticos ni protecciones de seguridad.
- Guardar el estado original y ofrecer una forma comprobable de restaurarlo antes de habilitar cambios.
- No hacer cambios ocultos ni automáticos en segundo plano.
- Evitar promesas de mejora sin mediciones reproducibles.

## Estado

El repositorio contiene la licencia MIT y la configuración inicial de Git para Rust. La aplicación y sus perfiles aún no están implementados.

## Licencia

Este proyecto se distribuye bajo la licencia [MIT](LICENSE).

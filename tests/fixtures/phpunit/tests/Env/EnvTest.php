<?php

declare(strict_types=1);

namespace Fixture\Tests;

use PHPUnit\Framework\TestCase;

/**
 * Montre dans sa sortie les variables d'environnement reçues : sert à vérifier que
 * `[phpunit.env]` et les variables d'une demande arrivent au processus PHP.
 * Suite « env », à lancer seule.
 */
final class EnvTest extends TestCase
{
    public function testShowsTheEnvironment(): void
    {
        foreach (['APP_ENV', 'FROM_SETTINGS', 'FROM_REQUEST', 'OVERRIDDEN'] as $name) {
            $value = getenv($name);
            echo $name . '=' . ($value === false ? '(absent)' : $value) . "\n";
        }
        $this->assertTrue(true);
    }
}

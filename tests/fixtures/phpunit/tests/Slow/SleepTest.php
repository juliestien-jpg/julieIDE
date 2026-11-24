<?php

declare(strict_types=1);

namespace Fixture\Tests;

use PHPUnit\Framework\TestCase;

final class SleepTest extends TestCase
{
    public function testSleepsThirtySeconds(): void
    {
        // Les essais de julie-phpunit y lisent le numéro du processus pour vérifier
        // qu'il a bien disparu après un arrêt.
        $pidFile = getenv('JULIE_PID_FILE');
        if ($pidFile !== false && $pidFile !== '') {
            file_put_contents($pidFile, (string) getmypid());
        }
        sleep(30);
        $this->assertTrue(true);
    }
}

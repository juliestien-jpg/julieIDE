<?php

declare(strict_types=1);

namespace Fixture\Tests;

use PHPUnit\Framework\TestCase;

final class CrashTest extends TestCase
{
    public function testBeforeTheCrash(): void
    {
        $this->assertTrue(true);
    }

    public function testExitsTheProcess(): void
    {
        echo "avant exit\n";
        exit(1);
    }

    public function testNeverReached(): void
    {
        $this->assertTrue(true);
    }
}

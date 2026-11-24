<?php

declare(strict_types=1);

namespace Fixture\Tests;

use PHPUnit\Framework\Attributes\DataProvider;
use PHPUnit\Framework\TestCase;

final class CalculatorTest extends TestCase
{
    public function testAdditionPasses(): void
    {
        $this->assertSame(4, 2 + 2);
    }

    public function testSecondAssertionPasses(): void
    {
        $this->assertTrue(true);
        $this->assertCount(3, [1, 2, 3]);
    }

    public function testEqualsFailsWithExpectedAndActual(): void
    {
        $this->assertEquals('attendu', 'obtenu');
    }

    public function testMultilineEqualsFails(): void
    {
        $this->assertEquals("ligne 1\nligne 2\nligne 3", "ligne 1\nligne deux\nligne 3");
    }

    public function testThrowsAnException(): void
    {
        throw new \RuntimeException("boum | avec 'apostrophes' et [crochets]");
    }

    public function testIsSkipped(): void
    {
        $this->markTestSkipped("pas sur cette machine");
    }

    public function testIsIncomplete(): void
    {
        $this->markTestIncomplete("à finir");
    }

    public function testIsRisky(): void
    {
        // Aucune assertion : PHPUnit le déclare risqué.
    }

    /**
     * @dataProvider additionProvider
     */
    #[DataProvider('additionProvider')]
    public function testAdditionWithDataSet(int $a, int $b, int $expected): void
    {
        $this->assertSame($expected, $a + $b);
    }

    public static function additionProvider(): array
    {
        return [
            'zéro plus zéro' => [0, 0, 0],
            "l'apostrophe | la barre [et] les crochets" => [1, 1, 2],
            'échoue' => [1, 2, 4],
        ];
    }

    public function testWritesOnOutput(): void
    {
        echo "texte écrit par le test\n";
        $this->assertTrue(true);
    }

    public function testIsSlow(): void
    {
        sleep(1);
        $this->assertTrue(true);
    }
}
